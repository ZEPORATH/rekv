use anyhow::{Context, Result};
use redis::AsyncCommands;
use serde_json::Value;

/// Redis client wrapper for pub/sub notifications.
/// Only used for publishing events, not as a datastore.
pub struct RedisNotifier {
    client: redis::Client,
    channel: String,
}

impl RedisNotifier {
    /// Create a new RedisNotifier.
    /// `redis_url` should be in the format "redis://localhost:6379"
    /// `channel` is the Redis channel name for notifications.
    pub fn new(redis_url: &str, channel: String) -> Result<Self> {
        let client = redis::Client::open(redis_url)
            .context("Failed to create Redis client")?;
        
        Ok(Self { client, channel })
    }

    /// Publish a config update event to Redis.
    /// The event contains: path, old_value, new_value.
    /// 
    /// NOTE: This is fire-and-forget. If Redis is down, the error is logged
    /// but doesn't block the config update. Consider using a background task
    /// with retries if you need guaranteed delivery.
    pub async fn publish_update(&self, path: &str, old_value: Option<&Value>, new_value: &Value) -> Result<()> {
        let mut conn = self.client.get_async_connection().await
            .context("Failed to get Redis connection")?;

        let event = serde_json::json!({
            "path": path,
            "old_value": old_value,
            "new_value": new_value,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        let payload = serde_json::to_string(&event)
            .context("Failed to serialize event")?;

        conn.publish::<_, _, ()>(&self.channel, payload).await
            .context("Failed to publish to Redis channel")?;

        Ok(())
    }
}
