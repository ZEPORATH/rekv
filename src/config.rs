use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;
use std::sync::RwLock;

/// In-memory hierarchical configuration store.
/// Uses RwLock for single-writer, multi-reader access.
pub struct ConfigStore {
    config: RwLock<Value>,
    file_path: String,
}

impl ConfigStore {
    /// Create a new ConfigStore with the given file path.
    pub fn new(file_path: String) -> Self {
        Self {
            config: RwLock::new(Value::Object(serde_json::Map::new())),
            file_path,
        }
    }

    /// Load configuration from disk.
    /// If the file doesn't exist, starts with an empty object.
    pub fn load(&self) -> Result<()> {
        let path = Path::new(&self.file_path);
        
        if !path.exists() {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .context("Failed to create config directory")?;
            }
            // Start with empty config
            return Ok(());
        }

        let content = std::fs::read_to_string(path)
            .context("Failed to read config file")?;
        
        let value: Value = serde_json::from_str(&content)
            .context("Failed to parse config JSON")?;

        *self.config.write().unwrap() = value;
        Ok(())
    }

    /// Save configuration to disk atomically (temp file + rename).
    /// This ensures we don't corrupt the config file if the process crashes mid-write.
    pub fn save(&self) -> Result<()> {
        let config = self.config.read().unwrap();
        let json = serde_json::to_string_pretty(&*config)
            .context("Failed to serialize config")?;

        let path = Path::new(&self.file_path);
        let temp_path = format!("{}.tmp", self.file_path);

        // Write to temp file first
        std::fs::write(&temp_path, json)
            .context("Failed to write temp config file")?;

        // Atomic rename (POSIX guarantees this is atomic)
        std::fs::rename(&temp_path, path)
            .context("Failed to rename temp file to config file")?;

        Ok(())
    }

    /// Get the full configuration.
    pub fn get_full(&self) -> Value {
        self.config.read().unwrap().clone()
    }

    /// Get a value at a given path (e.g., "device1.gpio.17.value").
    /// Returns None if the path doesn't exist.
    pub fn get_path(&self, path: &str) -> Option<Value> {
        let config = self.config.read().unwrap();
        get_json_path(&config, path)
    }

    /// Set a value at a given path.
    /// Returns the old value if it existed, None otherwise.
    /// 
    /// NOTE: This acquires a write lock, so all readers are blocked during the update.
    /// For single-writer scenarios, this is acceptable. If you need higher throughput,
    /// consider using Arc<RwLock<Value>> with copy-on-write patterns, but that adds complexity.
    pub fn set_path(&self, path: &str, value: Value) -> Result<Option<Value>> {
        let mut config = self.config.write().unwrap();
        let old_value = get_json_path(&config, path);
        
        set_json_path(&mut config, path, value.clone())
            .context("Failed to set config path")?;

        // Persist to disk
        drop(config); // Release lock before I/O
        self.save()?;

        Ok(old_value)
    }

    /// Get a reference to the inner RwLock for advanced use cases.
    /// Use with caution - prefer the public methods.
    pub fn inner(&self) -> &RwLock<Value> {
        &self.config
    }
}

/// Navigate a JSON value using a dot-separated path.
/// Returns None if any part of the path doesn't exist.
fn get_json_path(value: &Value, path: &str) -> Option<Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;

    for part in parts {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) => {
                let idx: usize = part.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }

    Some(current.clone())
}

/// Set a JSON value at a dot-separated path, creating intermediate objects as needed.
/// Returns an error if the path is invalid (e.g., trying to set a key on an array).
fn set_json_path(root: &mut Value, path: &str, value: Value) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    
    if parts.is_empty() {
        return Err(anyhow::anyhow!("Empty path"));
    }

    let mut current = root;
    
    // Navigate to the parent of the target
    for part in &parts[..parts.len() - 1] {
        match current {
            Value::Object(map) => {
                // Get or create the next level
                if !map.contains_key(*part) {
                    map.insert(part.to_string(), Value::Object(serde_json::Map::new()));
                }
                current = map.get_mut(*part).unwrap();
            }
            Value::Array(arr) => {
                let idx: usize = part.parse()
                    .context("Cannot use non-numeric key on array")?;
                if idx >= arr.len() {
                    return Err(anyhow::anyhow!("Array index out of bounds"));
                }
                current = &mut arr[idx];
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Cannot set path on non-object/non-array value"
                ));
            }
        }
    }

    // Set the final value
    let final_key = parts.last().unwrap();
    match current {
        Value::Object(map) => {
            map.insert(final_key.to_string(), value);
        }
        Value::Array(arr) => {
            let idx: usize = final_key.parse()
                .context("Cannot use non-numeric key on array")?;
            if idx >= arr.len() {
                return Err(anyhow::anyhow!("Array index out of bounds"));
            }
            arr[idx] = value;
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Cannot set value on non-object/non-array"
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_path() {
        let json = json!({
            "device1": {
                "gpio": {
                    "17": {
                        "value": 1
                    }
                }
            }
        });

        assert_eq!(get_json_path(&json, "device1.gpio.17.value"), Some(json!(1)));
        assert_eq!(get_json_path(&json, "device1.gpio"), Some(json!({"17": {"value": 1}})));
        assert_eq!(get_json_path(&json, "nonexistent"), None);
    }

    #[test]
    fn test_set_path() {
        let mut json = json!({});
        set_json_path(&mut json, "device1.gpio.17.value", json!(1)).unwrap();
        
        assert_eq!(get_json_path(&json, "device1.gpio.17.value"), Some(json!(1)));
    }
}
