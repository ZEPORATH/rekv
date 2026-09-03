use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

use crate::constants::{
    DEFAULT_BROADCAST_CHANNEL_CAPACITY, PATH_SEPARATOR_CHAR, ROOT_PATH,
};
use crate::storage::normalize_path;

/// Event payload sent to subscribers when configuration changes.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeEvent {
    /// Canonical path of the changed node.
    pub path: String,
    /// Old JSON value string if known.
    pub old_value: Option<String>,
    /// New JSON value string.
    pub new_value: String,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
}

impl ChangeEvent {
    pub fn new(path: impl Into<String>, old_val: Option<String>, new_val: impl Into<String>) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            path: path.into(),
            old_value: old_val,
            new_value: new_val.into(),
            timestamp_ms,
        }
    }
}

/// Pub/Sub engine supporting hierarchical bubble-up notifications.
pub struct PubSubEngine {
    /// Map of watched paths to broadcast senders.
    channels: RwLock<HashMap<String, broadcast::Sender<ChangeEvent>>>,
    /// Capacity of each broadcast channel.
    channel_capacity: usize,
}

impl Default for PubSubEngine {
    fn default() -> Self {
        Self::new(DEFAULT_BROADCAST_CHANNEL_CAPACITY)
    }
}

impl PubSubEngine {
    pub fn new(channel_capacity: usize) -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            channel_capacity,
        }
    }

    /// Subscribe to changes on a path or root /.
    pub fn subscribe(&self, path: &str) -> broadcast::Receiver<ChangeEvent> {
        let norm_path = normalize_path(path);

        // Check for existing channel under read lock.
        {
            let channels = self.channels.read().unwrap();
            if let Some(sender) = channels.get(&norm_path) {
                return sender.subscribe();
            }
        }

        // Insert new channel under write lock.
        let mut channels = self.channels.write().unwrap();
        let sender = channels
            .entry(norm_path)
            .or_insert_with(|| broadcast::channel(self.channel_capacity).0);

        sender.subscribe()
    }

    /// Publish an event to the exact path and all parent paths.
    pub fn publish(&self, event: ChangeEvent) -> usize {
        let ancestor_paths = bubble_up_paths(&event.path);
        let channels = self.channels.read().unwrap();
        let mut notified_count = 0;

        for ancestor in &ancestor_paths {
            if let Some(sender) = channels.get(ancestor) {
                if sender.send(event.clone()).is_ok() {
                    notified_count += 1;
                }
            }
        }

        notified_count
    }

    /// Return count of active channels.
    pub fn active_channel_count(&self) -> usize {
        self.channels.read().unwrap().len()
    }
}

/// Return all ancestor paths from a node up to root / for bubble-up notifications.
pub fn bubble_up_paths(path: &str) -> Vec<String> {
    let norm = normalize_path(path);
    if norm == ROOT_PATH {
        return vec![ROOT_PATH.to_string()];
    }

    let mut paths = Vec::new();
    let mut curr = norm.as_str();

    paths.push(curr.to_string());

    while let Some(pos) = curr.rfind(PATH_SEPARATOR_CHAR) {
        if pos == 0 {
            paths.push(ROOT_PATH.to_string());
            break;
        } else {
            curr = &curr[..pos];
            paths.push(curr.to_string());
        }
    }

    paths
}

