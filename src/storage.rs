use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::constants::{
    JSON_NULL, PATH_SEPARATOR_CHAR, PRIMARY_KEY_ATTRIBUTE, ROOT_PATH, TEMP_FILE_SUFFIX,
};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeltaEntry {
    Set {
        value: Value,
        #[serde(default)]
        typed_lists: BTreeMap<String, String>,
    },
    Unused,
}

/// Storage errors.
#[derive(Debug)]
pub enum StorageError {
    Io(io::Error),
    Json(serde_json::Error),
    NotFound(String),
    InvalidPath(String),
    InvalidValue(String),
    NoFilePath,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Io(e) => write!(f, "I/O error: {}", e),
            StorageError::Json(e) => write!(f, "JSON serialization error: {}", e),
            StorageError::NotFound(p) => write!(f, "Path not found: {}", p),
            StorageError::InvalidPath(p) => write!(f, "Invalid path: {}", p),
            StorageError::InvalidValue(v) => write!(f, "Invalid value: {}", v),
            StorageError::NoFilePath => write!(f, "No file path configured for persistence"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<io::Error> for StorageError {
    fn from(e: io::Error) -> Self {
        StorageError::Io(e)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Json(e)
    }
}

/// Storage engine for hierarchical JSON configuration.
#[derive(Debug, Clone)]
pub struct Store {
    /// In-memory JSON tree.
    tree: Value,

    /// Original settings loaded from the configured JSON file.
    base_tree: Value,

    /// Typed-list metadata from the original settings.
    base_typed_lists: BTreeMap<String, String>,

    /// Sparse path-level changes relative to the original settings.
    delta: BTreeMap<String, DeltaEntry>,

    /// Flat map of canonical path to leaf value.
    flat: BTreeMap<String, Value>,

    /// Index for attribute filtering: (path, key, val) -> indices.
    pred_index: HashMap<(String, String, String), Vec<usize>>,

    /// Index for primary keys: (path, id) -> index.
    id_index: HashMap<(String, String), usize>,

    /// Arrays whose items are all objects and can be selected by id.
    object_array_paths: HashSet<String>,

    /// Path to file on disk for persistence.
    file_path: Option<PathBuf>,

    /// Sibling `_delta.json` path.
    delta_file_path: Option<PathBuf>,

    /// Runtime types for arrays that JSON alone cannot distinguish.
    typed_lists: BTreeMap<String, String>,
}

impl Store {
    /// Create an empty Store.
    pub fn empty() -> Self {
        Self::from_value(Value::Object(serde_json::Map::new()), None)
    }

    /// Create a Store from a JSON value and optional file path.
    pub fn from_value(tree: Value, file_path: Option<PathBuf>) -> Self {
        let delta_file_path = file_path.as_deref().map(delta_path_for);
        let mut store = Store {
            tree: tree.clone(),
            base_tree: tree,
            base_typed_lists: BTreeMap::new(),
            delta: BTreeMap::new(),
            flat: BTreeMap::new(),
            pred_index: HashMap::new(),
            id_index: HashMap::new(),
            object_array_paths: HashSet::new(),
            file_path,
            delta_file_path,
            typed_lists: BTreeMap::new(),
        };
        store.rebuild_indices();
        store
    }

    /// Load configuration from a JSON file.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        let path_buf = path.as_ref().to_path_buf();
        let (tree, typed_lists) = if path_buf.exists() {
            let content = fs::read_to_string(&path_buf)?;
            let document: Value = serde_json::from_str(&content)?;
            decode_persisted_document(document)?
        } else {
            (Value::Object(serde_json::Map::new()), BTreeMap::new())
        };
        let mut store = Self::from_value(tree.clone(), Some(path_buf));
        store.base_typed_lists = typed_lists.clone();
        store.typed_lists = typed_lists;
        if let Some(delta_path) = store.delta_file_path.as_ref() {
            if delta_path.exists() {
                let delta_document: Value = serde_json::from_slice(&fs::read(delta_path)?)?;
                store.delta = serde_json::from_value(delta_document)?;
                store.rebuild_effective_tree()?;
            }
        }
        crate::config_value::ConfigValue::from_json_with_types(
            store.tree.clone(),
            "/",
            &store.typed_lists,
        )
        .map_err(StorageError::InvalidValue)?;
        Ok(store)
    }

    /// Return persistence file path if configured.
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    pub fn delta_file_path(&self) -> Option<&Path> {
        self.delta_file_path.as_deref()
    }

    pub fn delta_entries(&self) -> &BTreeMap<String, DeltaEntry> {
        &self.delta
    }

    pub fn resolve_base_path(&self, path: &str) -> Result<Option<String>, String> {
        let query = crate::path_parser::QueryPath::parse(path)?;
        let base = Store::from_value(self.base_tree.clone(), None);
        Ok(crate::path_resolver::resolve_paths(&query, &base)
            .map_err(|error| error.to_string())?
            .into_iter()
            .next())
    }

    /// Set persistence file path.
    pub fn set_file_path<P: Into<PathBuf>>(&mut self, path: P) {
        let path = path.into();
        self.delta_file_path = Some(delta_path_for(&path));
        self.file_path = Some(path);
    }

    /// Reference to underlying JSON tree.
    pub fn tree(&self) -> &Value {
        &self.tree
    }

    pub fn typed_lists(&self) -> &BTreeMap<String, String> {
        &self.typed_lists
    }

    pub fn set_typed_lists(&mut self, types: BTreeMap<String, String>) {
        let mut roots = BTreeMap::new();
        for (path, kind) in types {
            let path = normalize_path(&path);
            self.typed_lists.insert(path.clone(), kind);
            let root = self
                .delta
                .keys()
                .filter(|candidate| {
                    path == candidate.as_str() || path.starts_with(&format!("{}/", candidate))
                })
                .max_by_key(|candidate| candidate.len())
                .cloned()
                .unwrap_or_else(|| path.clone());
            roots.insert(root, ());
        }
        for root in roots.keys() {
            self.refresh_delta_set(root);
        }
    }

    /// Rebuild all secondary indices from the JSON tree.
    pub fn rebuild_indices(&mut self) {
        self.flat.clear();
        self.pred_index.clear();
        self.id_index.clear();
        self.object_array_paths.clear();

        Self::index_recursive(
            &self.tree,
            "",
            &mut self.flat,
            &mut self.pred_index,
            &mut self.id_index,
            &mut self.object_array_paths,
        );
    }

    fn index_recursive(
        node: &Value,
        path: &str,
        flat: &mut BTreeMap<String, Value>,
        pred_index: &mut HashMap<(String, String, String), Vec<usize>>,
        id_index: &mut HashMap<(String, String), usize>,
        object_array_paths: &mut HashSet<String>,
    ) {
        match node {
            Value::Object(map) => {
                if map.is_empty() {
                    if !path.is_empty() {
                        flat.insert(path.to_string(), node.clone());
                    }
                } else {
                    for (k, v) in map {
                        let child_path = if path.is_empty() {
                            format!("/{}", k)
                        } else {
                            format!("{}/{}", path, k)
                        };
                        Self::index_recursive(
                            v,
                            &child_path,
                            flat,
                            pred_index,
                            id_index,
                            object_array_paths,
                        );
                    }
                }
            }
            Value::Array(items) => {
                if items.iter().all(Value::is_object) {
                    object_array_paths.insert(path.to_string());
                }
                if items.is_empty() {
                    if !path.is_empty() {
                        flat.insert(path.to_string(), node.clone());
                    }
                } else {
                    for (i, item) in items.iter().enumerate() {
                        let child_path = format!("{}/{}", path, i);

                        if let Value::Object(item_map) = item {
                            for (attr_key, attr_val) in item_map {
                                if let Some(val_str) = primitive_to_string(attr_val) {
                                    // Index object-array ids for runtime selectors.
                                    if attr_key == PRIMARY_KEY_ATTRIBUTE {
                                        if let Some(id) = attr_val.as_str() {
                                            id_index
                                                .entry((path.to_string(), id.to_string()))
                                                .or_insert(i);
                                        }
                                    }

                                    // Index attribute for filtering
                                    pred_index
                                        .entry((path.to_string(), attr_key.clone(), val_str))
                                        .or_default()
                                        .push(i);
                                }
                            }
                        }

                        Self::index_recursive(
                            item,
                            &child_path,
                            flat,
                            pred_index,
                            id_index,
                            object_array_paths,
                        );
                    }
                }
            }
            _ => {
                flat.insert(path.to_string(), node.clone());
            }
        }
    }

    /// Read a leaf value by canonical path.
    pub fn get_leaf(&self, canonical_path: &str) -> Option<&Value> {
        let norm = normalize_path(canonical_path);
        self.flat.get(norm.as_str())
    }

    /// Check if a path exists.
    pub fn exists(&self, canonical_path: &str) -> bool {
        let norm = normalize_path(canonical_path);
        if norm.is_empty() || norm == "/" {
            return true;
        }
        self.flat.contains_key(norm.as_str()) || self.has_subtree(&norm)
    }

    /// Check if a path has child keys.
    pub fn has_subtree(&self, prefix: &str) -> bool {
        let norm = normalize_path(prefix);
        let prefix_slash = if norm == "/" {
            "/".to_string()
        } else {
            format!("{}/", norm)
        };
        self.flat
            .range(prefix_slash.clone()..)
            .next()
            .is_some_and(|(k, _)| k.starts_with(&prefix_slash))
    }

    /// Retrieve a whole subtree by path.
    pub fn get_subtree(&self, path: &str) -> Option<Value> {
        self.get_subtree_ref(path).cloned()
    }

    pub fn get_subtree_ref(&self, path: &str) -> Option<&Value> {
        let norm = normalize_path(path);
        if norm.is_empty() || norm == "/" {
            return Some(&self.tree);
        }

        // Walk tree along path segments
        let segments = parse_simple_segments(&norm);
        let mut curr = &self.tree;

        for seg in segments {
            match curr {
                Value::Object(map) => {
                    curr = map.get(seg)?;
                }
                Value::Array(arr) => {
                    let idx: usize = seg.parse().ok()?;
                    curr = arr.get(idx)?;
                }
                _ => return None,
            }
        }

        Some(curr)
    }

    /// Find array item index by primary key id.
    pub fn find_index_by_id(&self, array_path: &str, id: &str) -> Option<usize> {
        let norm = normalize_path(array_path);
        self.id_index.get(&(norm, id.to_string())).copied()
    }

    pub fn is_object_array(&self, array_path: &str) -> bool {
        self.object_array_paths
            .contains(&normalize_path(array_path))
    }

    /// Retrieve array item value by primary key id.
    pub fn get_item_by_id(&self, array_path: &str, id: &str) -> Option<Value> {
        let idx = self.find_index_by_id(array_path, id)?;
        let item_path = format!("{}/{}", normalize_path(array_path), idx);
        self.get_subtree(&item_path)
    }

    /// Find array item indices matching an attribute value.
    pub fn find_indices_by_predicate(
        &self,
        array_path: &str,
        attr_key: &str,
        attr_val: &str,
    ) -> Vec<usize> {
        let norm = normalize_path(array_path);
        self.pred_index
            .get(&(norm, attr_key.to_string(), attr_val.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Retrieve all array items matching an attribute value.
    pub fn get_items_by_predicate(
        &self,
        array_path: &str,
        attr_key: &str,
        attr_val: &str,
    ) -> Vec<Value> {
        let indices = self.find_indices_by_predicate(array_path, attr_key, attr_val);
        let norm = normalize_path(array_path);
        indices
            .into_iter()
            .filter_map(|idx| self.get_subtree(&format!("{}/{}", norm, idx)))
            .collect()
    }

    /// Scan all keys starting with a prefix in sorted order.
    pub fn scan_prefix(&self, prefix: &str) -> Vec<(&String, &Value)> {
        let norm = normalize_path(prefix);
        if norm.is_empty() || norm == "/" {
            return self.flat.iter().collect();
        }

        let prefix_slash = format!("{}/", norm);
        let mut results = Vec::new();

        if let Some(val) = self.flat.get(&norm) {
            results.push((self.flat.get_key_value(&norm).unwrap().0, val));
        }

        for (k, v) in self.flat.range(prefix_slash.clone()..) {
            if k.starts_with(&prefix_slash) {
                results.push((k, v));
            } else {
                break;
            }
        }

        results
    }

    /// Set a value at path and rebuild indices.
    pub fn set(&mut self, path: &str, new_value: Value) -> Result<(), StorageError> {
        let norm = normalize_path(path);
        if norm.is_empty() || norm == "/" {
            self.tree = new_value;
            self.typed_lists.clear();
            self.remove_delta_under(&norm);
            self.refresh_delta_set(&norm);
            self.rebuild_indices();
            return Ok(());
        }

        let segments = parse_simple_segments(&norm);
        if segments.is_empty() {
            return Err(StorageError::InvalidPath(path.to_string()));
        }

        Self::set_recursive(&mut self.tree, &segments, new_value)?;
        self.remove_typed_lists_under(&norm);
        self.remove_delta_under(&norm);
        let delta_root = self.delta_ancestor_or(&norm);
        self.refresh_delta_set(&delta_root);
        self.rebuild_indices();
        Ok(())
    }

    /// Delete a value at an object key or array index.
    pub fn delete(&mut self, path: &str) -> Result<(), StorageError> {
        let norm = normalize_path(path);
        if norm == ROOT_PATH {
            return Err(StorageError::InvalidPath(
                "cannot delete the root".to_string(),
            ));
        }

        let segments = parse_simple_segments(&norm);
        if segments.is_empty() {
            return Err(StorageError::InvalidPath(path.to_string()));
        }

        let base_value = get_value_from_tree(&self.base_tree, &norm);
        let parent_path = if segments.len() <= 1 {
            ROOT_PATH.to_string()
        } else {
            format!("/{}", segments[..segments.len() - 1].join("/"))
        };
        let deleted_array_index = segments
            .last()
            .and_then(|segment| segment.parse::<usize>().ok())
            .filter(|_| {
                self.get_subtree(&parent_path)
                    .is_some_and(|value| value.is_array())
            });
        Self::delete_recursive(&mut self.tree, &segments)?;
        self.remove_typed_lists_under(&norm);
        if let Some(index) = deleted_array_index {
            self.reindex_typed_lists_after_delete(&parent_path, index);
        }
        self.remove_delta_under(&norm);
        let delta_root = self.delta_ancestor_or(&norm);
        if delta_root != norm {
            self.refresh_delta_set(&delta_root);
        } else if base_value.is_some() {
            self.delta.insert(norm, DeltaEntry::Unused);
        } else {
            self.delta.remove(&norm);
        }
        self.rebuild_effective_tree()?;
        self.rebuild_indices();
        Ok(())
    }

    fn delta_ancestor_or(&self, path: &str) -> String {
        let path = normalize_path(path);
        self.delta
            .keys()
            .filter(|candidate| {
                path == candidate.as_str() || path.starts_with(&format!("{}/", candidate))
            })
            .max_by_key(|candidate| candidate.len())
            .cloned()
            .unwrap_or(path)
    }

    fn remove_delta_under(&mut self, path: &str) {
        let path = normalize_path(path);
        let prefix = format!("{}/", path.trim_end_matches('/'));
        self.delta
            .retain(|candidate, _| candidate != &path && !candidate.starts_with(&prefix));
    }

    fn typed_lists_under(&self, path: &str) -> BTreeMap<String, String> {
        let path = normalize_path(path);
        let prefix = format!("{}/", path.trim_end_matches('/'));
        self.typed_lists
            .iter()
            .filter(|(candidate, _)| candidate.as_str() == path || candidate.starts_with(&prefix))
            .map(|(candidate, kind)| (candidate.clone(), kind.clone()))
            .collect()
    }

    fn refresh_delta_set(&mut self, path: &str) {
        let path = normalize_path(path);
        let current = get_value_from_tree(&self.tree, &path);
        let base = get_value_from_tree(&self.base_tree, &path);
        let current_types = self.typed_lists_under(&path);
        let base_types: BTreeMap<_, _> = self
            .base_typed_lists
            .iter()
            .filter(|(candidate, _)| {
                candidate.as_str() == path
                    || candidate.starts_with(&format!("{}/", path.trim_end_matches('/')))
            })
            .map(|(candidate, kind)| (candidate.clone(), kind.clone()))
            .collect();

        if current == base && current_types == base_types {
            self.delta.remove(&path);
        } else if let Some(value) = current {
            self.delta.insert(
                path,
                DeltaEntry::Set {
                    value,
                    typed_lists: current_types,
                },
            );
        }
    }

    fn rebuild_effective_tree(&mut self) -> Result<(), StorageError> {
        self.tree = self.base_tree.clone();
        self.typed_lists = self.base_typed_lists.clone();
        let delta = self.delta.clone();
        for (path, entry) in &delta {
            match entry {
                DeltaEntry::Set { value, typed_lists } => {
                    let norm = normalize_path(path);
                    if norm == ROOT_PATH {
                        self.tree = value.clone();
                        self.typed_lists = typed_lists.clone();
                    } else {
                        let segments = parse_simple_segments(&norm);
                        Self::set_recursive(&mut self.tree, &segments, value.clone())?;
                        self.remove_typed_lists_under(&norm);
                        self.typed_lists.extend(typed_lists.clone());
                    }
                }
                DeltaEntry::Unused => {
                    if get_value_from_tree(&self.tree, path).is_some() {
                        let segments = parse_simple_segments(path);
                        Self::delete_recursive(&mut self.tree, &segments)?;
                        self.remove_typed_lists_under(path);
                    }
                }
            }
        }
        self.rebuild_indices();
        Ok(())
    }

    /// Remove the sparse change at a path and restore its original value.
    pub fn restore_delta(&mut self, path: &str) -> Result<(), StorageError> {
        let path = normalize_path(path);
        if self.delta.remove(&path).is_none() {
            return Err(StorageError::NotFound(path));
        }
        self.rebuild_effective_tree()
    }

    fn delete_recursive(curr: &mut Value, segments: &[&str]) -> Result<(), StorageError> {
        let (head, tail) = segments
            .split_first()
            .ok_or_else(|| StorageError::InvalidPath("empty path".to_string()))?;

        if tail.is_empty() {
            return match curr {
                Value::Object(map) => map
                    .remove(*head)
                    .map(|_| ())
                    .ok_or_else(|| StorageError::NotFound((*head).to_string())),
                Value::Array(items) => {
                    let index = head.parse::<usize>().map_err(|_| {
                        StorageError::InvalidPath(format!("Expected array index: {}", head))
                    })?;
                    if index < items.len() {
                        items.remove(index);
                        Ok(())
                    } else {
                        Err(StorageError::NotFound((*head).to_string()))
                    }
                }
                _ => Err(StorageError::InvalidPath(format!("No child at {}", head))),
            };
        }

        match curr {
            Value::Object(map) => {
                let child = map
                    .get_mut(*head)
                    .ok_or_else(|| StorageError::NotFound((*head).to_string()))?;
                Self::delete_recursive(child, tail)
            }
            Value::Array(items) => {
                let index = head.parse::<usize>().map_err(|_| {
                    StorageError::InvalidPath(format!("Expected array index: {}", head))
                })?;
                let child = items
                    .get_mut(index)
                    .ok_or_else(|| StorageError::NotFound((*head).to_string()))?;
                Self::delete_recursive(child, tail)
            }
            _ => Err(StorageError::InvalidPath(format!("No child at {}", head))),
        }
    }

    fn remove_typed_lists_under(&mut self, path: &str) {
        let prefix = format!("{}/", normalize_path(path).trim_end_matches('/'));
        let norm = normalize_path(path);
        self.typed_lists
            .retain(|typed_path, _| typed_path != &norm && !typed_path.starts_with(&prefix));
    }

    fn reindex_typed_lists_after_delete(&mut self, parent: &str, deleted_index: usize) {
        let prefix = if normalize_path(parent) == ROOT_PATH {
            ROOT_PATH.to_string()
        } else {
            format!("{}/", normalize_path(parent))
        };
        let mut updated = BTreeMap::new();
        for (path, kind) in std::mem::take(&mut self.typed_lists) {
            let Some(suffix) = path.strip_prefix(&prefix) else {
                updated.insert(path, kind);
                continue;
            };
            let (index_text, remainder) = suffix.split_once('/').unwrap_or((suffix, ""));
            let Ok(index) = index_text.parse::<usize>() else {
                updated.insert(path, kind);
                continue;
            };
            if index == deleted_index {
                continue;
            }
            let shifted = if index > deleted_index {
                index - 1
            } else {
                index
            };
            let tail = if remainder.is_empty() {
                String::new()
            } else {
                format!("/{}", remainder)
            };
            updated.insert(format!("{}{}{}", prefix, shifted, tail), kind);
        }
        self.typed_lists = updated;
    }

    /// List object keys or array element identifiers at a path.
    pub fn list_children(&self, path: &str) -> Result<Vec<String>, StorageError> {
        let norm = normalize_path(path);
        let value = self
            .get_subtree(&norm)
            .ok_or_else(|| StorageError::NotFound(norm.clone()))?;

        match value {
            Value::Object(map) => Ok(map.keys().cloned().collect()),
            Value::Array(items) => items
                .iter()
                .map(|item| {
                    if let Some(identifier) = item.get(PRIMARY_KEY_ATTRIBUTE) {
                        return identifier.as_str().map(str::to_owned).ok_or_else(|| {
                            StorageError::InvalidValue(format!(
                                "array element at {} must contain a string '{}' identifier",
                                norm, PRIMARY_KEY_ATTRIBUTE
                            ))
                        });
                    }
                    primitive_to_string(item).ok_or_else(|| {
                        StorageError::InvalidValue(format!(
                            "array element at {} must contain an '{}' identifier or be a scalar",
                            norm, PRIMARY_KEY_ATTRIBUTE
                        ))
                    })
                })
                .collect(),
            _ => Err(StorageError::InvalidValue(format!(
                "{} has no children",
                norm
            ))),
        }
    }

    fn set_recursive(
        curr: &mut Value,
        segments: &[&str],
        new_value: Value,
    ) -> Result<(), StorageError> {
        if segments.is_empty() {
            *curr = new_value;
            return Ok(());
        }

        let head = segments[0];
        let tail = &segments[1..];

        if let Ok(idx) = head.parse::<usize>() {
            match curr {
                Value::Array(arr) => {
                    if idx < arr.len() {
                        if tail.is_empty() {
                            arr[idx] = new_value;
                            Ok(())
                        } else {
                            Self::set_recursive(&mut arr[idx], tail, new_value)
                        }
                    } else if idx == arr.len() && tail.is_empty() {
                        arr.push(new_value);
                        Ok(())
                    } else {
                        Err(StorageError::InvalidPath(format!(
                            "Array index out of bounds: {}",
                            idx
                        )))
                    }
                }
                _ => Err(StorageError::InvalidPath(format!(
                    "Expected array at index {}",
                    head
                ))),
            }
        } else {
            match curr {
                Value::Object(map) => {
                    if tail.is_empty() {
                        map.insert(head.to_string(), new_value);
                        Ok(())
                    } else {
                        let entry = map
                            .entry(head.to_string())
                            .or_insert_with(|| Value::Object(serde_json::Map::new()));
                        Self::set_recursive(entry, tail, new_value)
                    }
                }
                _ => Err(StorageError::InvalidPath(format!(
                    "Expected object for field {}",
                    head
                ))),
            }
        }
    }

    /// Atomically persist configuration to disk.
    pub fn persist(&self) -> Result<(), StorageError> {
        self.delta_file_path
            .as_ref()
            .ok_or(StorageError::NoFilePath)
            .and_then(|path| {
                let document = serde_json::to_value(&self.delta)?;
                self.write_json_file(path, &document)
            })
    }

    fn write_json_file(&self, file_path: &Path, document: &Value) -> Result<(), StorageError> {
        if let Some(parent) = file_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let json_bytes = serde_json::to_vec_pretty(document)?;
        let parent = file_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty());
        let file_name = file_path
            .file_name()
            .ok_or_else(|| StorageError::InvalidPath(file_path.display().to_string()))?
            .to_string_lossy();
        let (tmp_path, mut file) = loop {
            let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = file_path.with_file_name(format!(
                "{}{}.{}.{}",
                file_name,
                TEMP_FILE_SUFFIX,
                std::process::id(),
                counter
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => break (candidate, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(StorageError::Io(error)),
            }
        };

        let result = (|| {
            file.write_all(&json_bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp_path, file_path)?;
            if let Some(parent) = parent {
                File::open(parent)?.sync_all()?;
            }
            Ok::<(), StorageError>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        result
    }

    /// Total count of indexed leaf nodes.
    pub fn leaf_count(&self) -> usize {
        self.flat.len()
    }
}

fn decode_persisted_document(
    document: Value,
) -> Result<(Value, BTreeMap<String, String>), StorageError> {
    let Some(metadata) = document.get("$rekv") else {
        return Ok((document, BTreeMap::new()));
    };
    let Some(version) = metadata.get("version").and_then(Value::as_u64) else {
        return Ok((document, BTreeMap::new()));
    };
    if version != 1 {
        return Err(StorageError::InvalidValue(format!(
            "unsupported settings metadata version {}",
            version
        )));
    }
    let data = document.get("data").cloned().ok_or_else(|| {
        StorageError::InvalidValue("settings metadata is missing data".to_string())
    })?;
    let typed_lists = metadata
        .get("typed_lists")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            StorageError::InvalidValue("typed_lists metadata must be an object".to_string())
        })?
        .iter()
        .map(|(path, kind)| {
            let kind = kind.as_str().ok_or_else(|| {
                StorageError::InvalidValue(format!("invalid type metadata at {}", path))
            })?;
            if kind != "str_list" && kind != "numeric_list" {
                return Err(StorageError::InvalidValue(format!(
                    "unsupported typed list '{}' at {}",
                    kind, path
                )));
            }
            Ok((normalize_path(path), kind.to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, StorageError>>()?;
    Ok((data, typed_lists))
}

fn delta_path_for(settings_path: &Path) -> PathBuf {
    settings_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("_delta.json")
}

fn get_value_from_tree(tree: &Value, path: &str) -> Option<Value> {
    let norm = normalize_path(path);
    if norm == ROOT_PATH {
        return Some(tree.clone());
    }
    let segments = parse_simple_segments(&norm);
    let mut current = tree;
    for segment in segments {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current.clone())
}

/// Normalize path to have a leading slash and no trailing slash.
pub fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == ROOT_PATH {
        return ROOT_PATH.to_string();
    }
    let mut s = String::with_capacity(trimmed.len() + 1);
    if !trimmed.starts_with(PATH_SEPARATOR_CHAR) {
        s.push(PATH_SEPARATOR_CHAR);
    }
    s.push_str(trimmed);
    while s.len() > 1 && s.ends_with(PATH_SEPARATOR_CHAR) {
        s.pop();
    }
    s
}

/// Split path into segment strings.
fn parse_simple_segments(path: &str) -> Vec<&str> {
    path.split(PATH_SEPARATOR_CHAR)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Convert a JSON primitive value to string for indexing.
fn primitive_to_string(val: &Value) -> Option<String> {
    match val {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => Some(JSON_NULL.to_string()),
        _ => None,
    }
}
