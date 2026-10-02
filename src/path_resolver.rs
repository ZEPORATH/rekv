use crate::constants::{JSON_NULL, PATH_SEPARATOR_CHAR, PRIMARY_KEY_ATTRIBUTE, ROOT_PATH};
use crate::path_parser::{Predicate, PredicateOp, PredicateValue, QueryPath, Segment};
use crate::storage::{normalize_path, Store};
use serde_json::Value;

/// Result of a single resolved node.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode {
    pub path: String,
    pub value: Value,
}

/// Result of a GET operation on a query path.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolveGetResult {
    /// No matching node was found.
    NotFound,
    /// Exactly one matching node found.
    Single(ResolvedNode),
    /// Multiple matching nodes found.
    Multiple(Vec<ResolvedNode>),
}

impl ResolveGetResult {
    /// Convert result to a JSON Value.
    pub fn to_json_value(self) -> Value {
        match self {
            ResolveGetResult::NotFound => Value::Null,
            ResolveGetResult::Single(node) => node.value,
            ResolveGetResult::Multiple(nodes) => {
                Value::Array(nodes.into_iter().map(|n| n.value).collect())
            }
        }
    }

    /// Number of matching nodes.
    pub fn match_count(&self) -> usize {
        match self {
            ResolveGetResult::NotFound => 0,
            ResolveGetResult::Single(_) => 1,
            ResolveGetResult::Multiple(v) => v.len(),
        }
    }
}

/// Result of a SET operation.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolveSetResult {
    /// List of paths that were updated.
    pub affected_paths: Vec<String>,
}

/// Resolves a query path for reading data.
pub fn resolve_get(query_str: &str, store: &Store) -> Result<ResolveGetResult, String> {
    let query = QueryPath::parse(query_str)?;

    // Handle root path /
    if query.is_root() {
        return Ok(ResolveGetResult::Single(ResolvedNode {
            path: ROOT_PATH.to_string(),
            value: store.tree().clone(),
        }));
    }

    let matching_paths = resolve_paths(&query, store);

    if matching_paths.is_empty() {
        Ok(ResolveGetResult::NotFound)
    } else if matching_paths.len() == 1 {
        let path = matching_paths.into_iter().next().unwrap();
        if let Some(val) = get_value_at_path(&path, store) {
            Ok(ResolveGetResult::Single(ResolvedNode { path, value: val }))
        } else {
            Ok(ResolveGetResult::NotFound)
        }
    } else {
        let mut nodes = Vec::with_capacity(matching_paths.len());
        for path in matching_paths {
            if let Some(value) = get_value_at_path(&path, store) {
                nodes.push(ResolvedNode { path, value });
            }
        }
        if nodes.is_empty() {
            Ok(ResolveGetResult::NotFound)
        } else {
            Ok(ResolveGetResult::Multiple(nodes))
        }
    }
}

/// Resolves a query path and updates matching nodes with new_value.
pub fn resolve_set(
    query_str: &str,
    new_value: Value,
    store: &mut Store,
) -> Result<ResolveSetResult, String> {
    let query = QueryPath::parse(query_str)?;

    // Handle root path /
    if query.is_root() {
        store.set(ROOT_PATH, new_value).map_err(|e| e.to_string())?;
        return Ok(ResolveSetResult {
            affected_paths: vec![ROOT_PATH.to_string()],
        });
    }

    // Resolve existing matching paths
    let matching_paths = resolve_paths(&query, store);

    if matching_paths.is_empty() {
        // If no match, check if query is a simple static path to create
        if is_pure_static_path(&query) {
            let static_path = build_static_path(&query);
            store
                .set(&static_path, new_value)
                .map_err(|e| e.to_string())?;
            return Ok(ResolveSetResult {
                affected_paths: vec![static_path],
            });
        }
        return Err(format!("No matching nodes found for path: {}", query_str));
    }

    let mut affected_paths = Vec::with_capacity(matching_paths.len());
    for path in matching_paths {
        store
            .set(&path, new_value.clone())
            .map_err(|e| e.to_string())?;
        affected_paths.push(path);
    }

    Ok(ResolveSetResult { affected_paths })
}

/// Find all matching paths in the store for a query.
pub fn resolve_paths(query: &QueryPath, store: &Store) -> Vec<String> {
    let mut current_paths = vec![ROOT_PATH.to_string()];

    for (seg_idx, segment) in query.segments.iter().enumerate() {
        let is_last_segment = seg_idx == query.segments.len() - 1;
        let mut next_paths = Vec::new();

        for current_path in &current_paths {
            match segment {
                Segment::Key(name) => {
                    let child_path = append_segment(current_path, name);
                    if store.exists(&child_path) {
                        next_paths.push(child_path);
                    }
                }
                Segment::Index(idx) => {
                    let child_path = append_segment(current_path, &idx.to_string());
                    if store.exists(&child_path) {
                        next_paths.push(child_path);
                    }
                }
                Segment::Id(id_val) => {
                    // Fast primary key lookup
                    if let Some(idx) = store.find_index_by_id(current_path, id_val) {
                        let child_path = append_segment(current_path, &idx.to_string());
                        if store.exists(&child_path) {
                            next_paths.push(child_path);
                        }
                    }
                }
                Segment::Predicates(preds) => {
                    // Evaluate predicates on array elements
                    if let Some(Value::Array(items)) = store.get_subtree(current_path) {
                        // Use index if first predicate is string equality
                        let candidate_indices: Vec<usize> = if let Some(first_pred) = preds.first() {
                            if first_pred.op == PredicateOp::Eq {
                                if let PredicateValue::String(s) = &first_pred.value {
                                    if first_pred.key == PRIMARY_KEY_ATTRIBUTE {
                                        store
                                            .find_index_by_id(current_path, s)
                                            .into_iter()
                                            .collect()
                                    } else {
                                        store.find_indices_by_predicate(
                                            current_path,
                                            &first_pred.key,
                                            s,
                                        )
                                    }
                                } else {
                                    (0..items.len()).collect()
                                }
                            } else {
                                (0..items.len()).collect()
                            }
                        } else {
                            (0..items.len()).collect()
                        };

                        // Check all predicates on candidate items
                        for idx in candidate_indices {
                            if idx < items.len() {
                                let item = &items[idx];
                                if preds.iter().all(|pred| matches_predicate(item, pred)) {
                                    let child_path =
                                        append_segment(current_path, &idx.to_string());
                                    next_paths.push(child_path);
                                }
                            }
                        }
                    }
                }
                Segment::Wildcard => {
                    // Match immediate children
                    if let Some(val) = store.get_subtree(current_path) {
                        match val {
                            Value::Object(map) => {
                                for key in map.keys() {
                                    next_paths.push(append_segment(current_path, key));
                                }
                            }
                            Value::Array(arr) => {
                                for i in 0..arr.len() {
                                    next_paths.push(append_segment(current_path, &i.to_string()));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Segment::RecursiveWildcard => {
                    if is_last_segment {
                        let prefix_matches = store.scan_prefix(current_path);
                        for (k, _) in prefix_matches {
                            if k != current_path {
                                next_paths.push(k.clone());
                            }
                        }
                    } else {
                        let descendants = get_all_intermediate_and_leaf_paths(store, current_path);
                        next_paths.extend(descendants);
                    }
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

    current_paths
}

// Helper: collect all intermediate and leaf descendant paths under base_path.
fn get_all_intermediate_and_leaf_paths(store: &Store, base_path: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut all_paths = BTreeSet::new();
    let norm_base = normalize_path(base_path);

    all_paths.insert(norm_base.clone());

    let prefix_slash = if norm_base == "/" {
        "/".to_string()
    } else {
        format!("{}/", norm_base)
    };

    for (k, _) in store.scan_prefix(&norm_base) {
        if k.starts_with(&prefix_slash) {
            let mut curr = String::new();
            for part in k.split('/') {
                if part.is_empty() {
                    continue;
                }
                curr.push('/');
                curr.push_str(part);
                if curr.starts_with(&prefix_slash) || curr == norm_base {
                    all_paths.insert(curr.clone());
                }
            }
        }
    }

    all_paths.into_iter().collect()
}

// Append child segment to parent path string.
fn append_segment(parent: &str, child: &str) -> String {
    let norm_parent = normalize_path(parent);
    if norm_parent == ROOT_PATH {
        format!("{}{}", ROOT_PATH, child)
    } else {
        format!("{}{}{}", norm_parent, PATH_SEPARATOR_CHAR, child)
    }
}

// Check if query path contains only static key and index segments.
fn is_pure_static_path(query: &QueryPath) -> bool {
    query.segments.iter().all(|s| matches!(s, Segment::Key(_) | Segment::Index(_)))
}

// Build path string from a static query path.
fn build_static_path(query: &QueryPath) -> String {
    let mut path = String::new();
    for seg in &query.segments {
        match seg {
            Segment::Key(k) => {
                path.push('/');
                path.push_str(k);
            }
            Segment::Index(i) => {
                path.push('/');
                path.push_str(&i.to_string());
            }
            _ => {}
        }
    }
    normalize_path(&path)
}

// Get value at path (tries leaf first, then subtree).
fn get_value_at_path(path: &str, store: &Store) -> Option<Value> {
    if let Some(leaf) = store.get_leaf(path) {
        Some(leaf.clone())
    } else {
        store.get_subtree(path)
    }
}

// Check if a JSON value matches a predicate.
fn matches_predicate(item: &Value, pred: &Predicate) -> bool {
    let map = match item {
        Value::Object(m) => m,
        _ => return false,
    };

    let actual_val = match map.get(&pred.key) {
        Some(v) => v,
        None => return false,
    };

    match (&pred.value, actual_val) {
        (PredicateValue::String(target), Value::String(actual)) => match pred.op {
            PredicateOp::Eq => actual == target,
            PredicateOp::NotEq => actual != target,
            PredicateOp::Gt => actual > target,
            PredicateOp::Gte => actual >= target,
            PredicateOp::Lt => actual < target,
            PredicateOp::Lte => actual <= target,
        },
        (PredicateValue::Number(target), Value::Number(actual)) => {
            if let Some(act_f64) = actual.as_f64() {
                compare_floats(act_f64, *target, pred.op)
            } else {
                false
            }
        }
        (PredicateValue::Bool(target), Value::Bool(actual)) => match pred.op {
            PredicateOp::Eq => actual == target,
            PredicateOp::NotEq => actual != target,
            _ => false,
        },
        (PredicateValue::Number(target), Value::String(s)) => {
            if let Ok(act_f64) = s.parse::<f64>() {
                compare_floats(act_f64, *target, pred.op)
            } else {
                false
            }
        }
        (PredicateValue::String(target), val) => {
            let actual_str = match val {
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => JSON_NULL.to_string(),
                _ => return false,
            };
            match pred.op {
                PredicateOp::Eq => &actual_str == target,
                PredicateOp::NotEq => &actual_str != target,
                _ => false,
            }
        }
        _ => false,
    }
}

// Compare floating point numbers with tolerance.
fn compare_floats(actual: f64, target: f64, op: PredicateOp) -> bool {
    match op {
        PredicateOp::Eq => (actual - target).abs() < 1e-9,
        PredicateOp::NotEq => (actual - target).abs() >= 1e-9,
        PredicateOp::Gt => actual > target,
        PredicateOp::Gte => actual >= target,
        PredicateOp::Lt => actual < target,
        PredicateOp::Lte => actual <= target,
    }
}

