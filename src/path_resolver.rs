use std::collections::BTreeSet;
use std::fmt;

use serde_json::Value;

use crate::constants::{PATH_SEPARATOR_CHAR, ROOT_PATH};
use crate::path_parser::{QueryPath, Segment, Selector};
use crate::storage::{normalize_path, StorageError, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    InvalidPath(String),
    NotFound(String),
    InvalidValue(String),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(message) | Self::NotFound(message) | Self::InvalidValue(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for ResolveError {}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode {
    pub path: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolveGetResult {
    NotFound,
    Single(ResolvedNode),
    Multiple(Vec<ResolvedNode>),
}

impl ResolveGetResult {
    pub fn to_json_value(self) -> Value {
        match self {
            Self::NotFound => Value::Null,
            Self::Single(node) => node.value,
            Self::Multiple(nodes) => {
                Value::Array(nodes.into_iter().map(|node| node.value).collect())
            }
        }
    }

    pub fn match_count(&self) -> usize {
        match self {
            Self::NotFound => 0,
            Self::Single(_) => 1,
            Self::Multiple(nodes) => nodes.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolveSetResult {
    pub affected_paths: Vec<String>,
}

pub fn resolve_get(query_str: &str, store: &Store) -> Result<ResolveGetResult, ResolveError> {
    let query = QueryPath::parse(query_str).map_err(ResolveError::InvalidPath)?;
    if query.is_root() {
        return Ok(ResolveGetResult::Single(ResolvedNode {
            path: ROOT_PATH.to_string(),
            value: store.tree().clone(),
        }));
    }

    let paths = resolve_paths(&query, store)?;
    let nodes: Vec<_> = paths
        .into_iter()
        .filter_map(|path| {
            get_value_at_path(&path, store).map(|value| ResolvedNode { path, value })
        })
        .collect();
    Ok(match nodes.len() {
        0 => ResolveGetResult::NotFound,
        1 => ResolveGetResult::Single(nodes.into_iter().next().unwrap()),
        _ => ResolveGetResult::Multiple(nodes),
    })
}

pub fn resolve_list(query_str: &str, store: &Store) -> Result<Vec<String>, ResolveError> {
    let query = QueryPath::parse(query_str).map_err(ResolveError::InvalidPath)?;
    let mut targets = query.segments.clone();
    let list_selected_object = matches!(targets.last(), Some(Segment::Wildcard));
    if list_selected_object {
        targets.pop();
    }
    let target_query = QueryPath { segments: targets };
    let paths = resolve_paths(&target_query, store)?;
    if paths.is_empty() {
        return Err(ResolveError::NotFound(format!(
            "configuration path does not exist: {}",
            query_str
        )));
    }

    let mut children = BTreeSet::new();
    for path in paths {
        if list_selected_object {
            let value = store.get_subtree(&path).ok_or_else(|| {
                ResolveError::NotFound(format!("configuration path does not exist: {}", path))
            })?;
            let object = value.as_object().ok_or_else(|| {
                ResolveError::InvalidPath(format!("wildcard requires an object at {}", path))
            })?;
            children.extend(object.keys().cloned());
        } else {
            let entries = store.list_children(&path).map_err(map_storage_error)?;
            children.extend(entries);
        }
    }
    Ok(children.into_iter().collect())
}

pub fn resolve_set(
    query_str: &str,
    new_value: Value,
    store: &mut Store,
) -> Result<ResolveSetResult, ResolveError> {
    let query = QueryPath::parse(query_str).map_err(ResolveError::InvalidPath)?;
    if query
        .segments
        .iter()
        .any(|segment| matches!(segment, Segment::Wildcard))
    {
        return Err(ResolveError::InvalidPath(
            "wildcards are supported for reads and LIST only".to_string(),
        ));
    }
    if query.is_root() {
        store
            .set(ROOT_PATH, new_value)
            .map_err(|error| ResolveError::InvalidPath(error.to_string()))?;
        return Ok(ResolveSetResult {
            affected_paths: vec![ROOT_PATH.to_string()],
        });
    }

    let paths = resolve_paths(&query, store)?;
    if paths.is_empty() {
        if query
            .segments
            .iter()
            .all(|segment| matches!(segment, Segment::Key(_)))
        {
            let path = build_static_path(&query);
            store
                .set(&path, new_value)
                .map_err(|error| ResolveError::InvalidPath(error.to_string()))?;
            return Ok(ResolveSetResult {
                affected_paths: vec![path],
            });
        }
        return Err(ResolveError::NotFound(format!(
            "configuration path does not exist: {}",
            query_str
        )));
    }

    let mut affected_paths = Vec::with_capacity(paths.len());
    for path in paths {
        store
            .set(&path, new_value.clone())
            .map_err(|error| ResolveError::InvalidPath(error.to_string()))?;
        affected_paths.push(path);
    }
    Ok(ResolveSetResult { affected_paths })
}

pub fn resolve_delete(query_str: &str, store: &mut Store) -> Result<String, ResolveError> {
    let query = QueryPath::parse(query_str).map_err(ResolveError::InvalidPath)?;
    if query.is_root()
        || query
            .segments
            .iter()
            .any(|segment| matches!(segment, Segment::Wildcard))
    {
        return Err(ResolveError::InvalidPath(
            "delete requires one non-root path".to_string(),
        ));
    }
    let mut paths = resolve_paths(&query, store)?;
    if paths.is_empty() {
        return Err(ResolveError::NotFound(format!(
            "configuration path does not exist: {}",
            query_str
        )));
    }
    if paths.len() != 1 {
        return Err(ResolveError::InvalidPath(
            "delete requires exactly one selected node".to_string(),
        ));
    }
    let path = paths.pop().unwrap();
    store.delete(&path).map_err(map_storage_error)?;
    Ok(path)
}

pub fn resolve_paths(query: &QueryPath, store: &Store) -> Result<Vec<String>, ResolveError> {
    let mut current_paths = vec![ROOT_PATH.to_string()];
    for segment in &query.segments {
        let mut next_paths = Vec::new();
        for current_path in &current_paths {
            match segment {
                Segment::Key(name) => {
                    let Some(current) = store.get_subtree_ref(current_path) else {
                        continue;
                    };
                    match current {
                        Value::Object(object) => {
                            if object.contains_key(name) {
                                next_paths.push(append_segment(current_path, name));
                            }
                        }
                        Value::Array(_) => {
                            return Err(ResolveError::InvalidPath(format!(
                                "array at {} requires an [id = ...] or [idx = ...] selector",
                                current_path
                            )))
                        }
                        _ => {}
                    }
                }
                Segment::Selector(Selector::Id(expected)) => {
                    if !store.exists(current_path) {
                        continue;
                    }
                    if !store.is_object_array(current_path) {
                        return Err(ResolveError::InvalidPath(format!(
                            "id selector requires an array of objects at {}",
                            current_path
                        )));
                    }
                    if let Some(index) = store.find_index_by_id(current_path, expected) {
                        next_paths.push(append_segment(current_path, &index.to_string()));
                    }
                }
                Segment::Selector(Selector::Index(index)) => {
                    let Some(current) = store.get_subtree_ref(current_path) else {
                        continue;
                    };
                    let array = current.as_array().ok_or_else(|| {
                        ResolveError::InvalidPath(format!(
                            "selector applied to non-array path {}",
                            current_path
                        ))
                    })?;
                    if *index < array.len() {
                        next_paths.push(append_segment(current_path, &index.to_string()));
                    }
                }
                Segment::Wildcard => {
                    let Some(current) = store.get_subtree_ref(current_path) else {
                        continue;
                    };
                    let object = current.as_object().ok_or_else(|| {
                        ResolveError::InvalidPath(format!(
                            "wildcard requires an object at {}",
                            current_path
                        ))
                    })?;
                    next_paths.extend(object.keys().map(|key| append_segment(current_path, key)));
                }
            }
        }
        next_paths.sort();
        next_paths.dedup();
        current_paths = next_paths;
        if current_paths.is_empty() {
            break;
        }
    }
    Ok(current_paths)
}

fn append_segment(parent: &str, child: &str) -> String {
    let parent = normalize_path(parent);
    if parent == ROOT_PATH {
        format!("{}{}", ROOT_PATH, child)
    } else {
        format!("{}{}{}", parent, PATH_SEPARATOR_CHAR, child)
    }
}

fn build_static_path(query: &QueryPath) -> String {
    let mut path = String::new();
    for segment in &query.segments {
        if let Segment::Key(key) = segment {
            path.push(PATH_SEPARATOR_CHAR);
            path.push_str(key);
        }
    }
    normalize_path(&path)
}

fn get_value_at_path(path: &str, store: &Store) -> Option<Value> {
    store
        .get_leaf(path)
        .cloned()
        .or_else(|| store.get_subtree(path))
}

fn map_storage_error(error: StorageError) -> ResolveError {
    match error {
        StorageError::NotFound(path) => ResolveError::NotFound(format!("path not found: {}", path)),
        StorageError::InvalidPath(path) => ResolveError::InvalidPath(path),
        StorageError::InvalidValue(value) => ResolveError::InvalidValue(value),
        StorageError::Io(error) => ResolveError::InvalidValue(error.to_string()),
        StorageError::Json(error) => ResolveError::InvalidValue(error.to_string()),
        StorageError::NoFilePath => ResolveError::InvalidValue("no settings path".to_string()),
    }
}
