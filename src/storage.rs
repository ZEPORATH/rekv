use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use serde_json::Value;

use crate::constants::{
    JSON_NULL, PATH_SEPARATOR_CHAR, PRIMARY_KEY_ATTRIBUTE, ROOT_PATH, TEMP_FILE_SUFFIX,
};

/// Storage errors.
#[derive(Debug)]
pub enum StorageError {
    Io(io::Error),
    Json(serde_json::Error),
    NotFound(String),
    InvalidPath(String),
    NoFilePath,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Io(e) => write!(f, "I/O error: {}", e),
            StorageError::Json(e) => write!(f, "JSON serialization error: {}", e),
            StorageError::NotFound(p) => write!(f, "Path not found: {}", p),
            StorageError::InvalidPath(p) => write!(f, "Invalid path: {}", p),
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

    /// Flat map of canonical path to leaf value.
    flat: BTreeMap<String, Value>,

    /// Index for attribute filtering: (path, key, val) -> indices.
    pred_index: HashMap<(String, String, String), Vec<usize>>,

    /// Index for primary keys: (path, id) -> index.
    id_index: HashMap<(String, String), usize>,

    /// Path to file on disk for persistence.
    file_path: Option<PathBuf>,
}

impl Store {
    /// Create an empty Store.
    pub fn empty() -> Self {
        Self::from_value(Value::Object(serde_json::Map::new()), None)
    }

    /// Create a Store from a JSON value and optional file path.
    pub fn from_value(tree: Value, file_path: Option<PathBuf>) -> Self {
        let mut store = Store {
            tree,
            flat: BTreeMap::new(),
            pred_index: HashMap::new(),
            id_index: HashMap::new(),
            file_path,
        };
        store.rebuild_indices();
        store
    }

    /// Load configuration from a JSON file.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        let path_buf = path.as_ref().to_path_buf();
        let content = fs::read_to_string(&path_buf)?;
        let tree: Value = serde_json::from_str(&content)?;
        Ok(Self::from_value(tree, Some(path_buf)))
    }

    /// Return persistence file path if configured.
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// Set persistence file path.
    pub fn set_file_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.file_path = Some(path.into());
    }

    /// Reference to underlying JSON tree.
    pub fn tree(&self) -> &Value {
        &self.tree
    }

    /// Rebuild all secondary indices from the JSON tree.
    pub fn rebuild_indices(&mut self) {
        self.flat.clear();
        self.pred_index.clear();
        self.id_index.clear();

        Self::index_recursive(
            &self.tree,
            "",
            &mut self.flat,
            &mut self.pred_index,
            &mut self.id_index,
        );
    }

    fn index_recursive(
        node: &Value,
        path: &str,
        flat: &mut BTreeMap<String, Value>,
        pred_index: &mut HashMap<(String, String, String), Vec<usize>>,
        id_index: &mut HashMap<(String, String), usize>,
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
                        Self::index_recursive(v, &child_path, flat, pred_index, id_index);
                    }
                }
            }
            Value::Array(items) => {
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
                                    // Index primary key #id
                                    if attr_key == PRIMARY_KEY_ATTRIBUTE {
                                        id_index.insert((path.to_string(), val_str.clone()), i);
                                    }

                                    // Index attribute for filtering
                                    pred_index
                                        .entry((path.to_string(), attr_key.clone(), val_str))
                                        .or_default()
                                        .push(i);
                                }
                            }
                        }

                        Self::index_recursive(item, &child_path, flat, pred_index, id_index);
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
        self.flat.range(prefix_slash.clone()..).next().map_or(false, |(k, _)| k.starts_with(&prefix_slash))
    }

    /// Retrieve a whole subtree by path.
    pub fn get_subtree(&self, path: &str) -> Option<Value> {
        let norm = normalize_path(path);
        if norm.is_empty() || norm == "/" {
            return Some(self.tree.clone());
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

        Some(curr.clone())
    }

    /// Find array item index by primary key id.
    pub fn find_index_by_id(&self, array_path: &str, id: &str) -> Option<usize> {
        let norm = normalize_path(array_path);
        self.id_index.get(&(norm, id.to_string())).copied()
    }

    /// Retrieve array item value by primary key id.
    pub fn get_item_by_id(&self, array_path: &str, id: &str) -> Option<Value> {
        let idx = self.find_index_by_id(array_path, id)?;
        let item_path = format!("{}/{}", normalize_path(array_path), idx);
        self.get_subtree(&item_path)
    }

    /// Find array item indices matching an attribute value.
    pub fn find_indices_by_predicate(&self, array_path: &str, attr_key: &str, attr_val: &str) -> Vec<usize> {
        let norm = normalize_path(array_path);
        self.pred_index
            .get(&(norm, attr_key.to_string(), attr_val.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Retrieve all array items matching an attribute value.
    pub fn get_items_by_predicate(&self, array_path: &str, attr_key: &str, attr_val: &str) -> Vec<Value> {
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
            self.rebuild_indices();
            return Ok(());
        }

        let segments = parse_simple_segments(&norm);
        if segments.is_empty() {
            return Err(StorageError::InvalidPath(path.to_string()));
        }

        Self::set_recursive(&mut self.tree, &segments, new_value)?;
        self.rebuild_indices();
        Ok(())
    }

    fn set_recursive(curr: &mut Value, segments: &[&str], new_value: Value) -> Result<(), StorageError> {
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
                        Err(StorageError::InvalidPath(format!("Array index out of bounds: {}", idx)))
                    }
                }
                _ => Err(StorageError::InvalidPath(format!("Expected array at index {}", head))),
            }
        } else {
            match curr {
                Value::Object(map) => {
                    if tail.is_empty() {
                        map.insert(head.to_string(), new_value);
                        Ok(())
                    } else {
                        let entry = map.entry(head.to_string()).or_insert_with(|| Value::Object(serde_json::Map::new()));
                        Self::set_recursive(entry, tail, new_value)
                    }
                }
                _ => Err(StorageError::InvalidPath(format!("Expected object for field {}", head))),
            }
        }
    }

    /// Atomically persist configuration to disk.
    pub fn persist(&self) -> Result<(), StorageError> {
        let file_path = self.file_path.as_ref().ok_or(StorageError::NoFilePath)?;

        if let Some(parent) = file_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let json_bytes = serde_json::to_vec_pretty(&self.tree)?;

        let tmp_path = format!(
            "{}{}.{}",
            file_path.to_string_lossy(),
            TEMP_FILE_SUFFIX,
            std::process::id()
        );
        let tmp_path_buf = PathBuf::from(&tmp_path);

        {
            let mut file = File::create(&tmp_path_buf)?;
            file.write_all(&json_bytes)?;
            file.sync_all()?;
        }

        fs::rename(&tmp_path_buf, file_path)?;

        Ok(())
    }

    /// Total count of indexed leaf nodes.
    pub fn leaf_count(&self) -> usize {
        self.flat.len()
    }
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
