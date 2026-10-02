use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rekv::config_service::ConfigService;
use rekv::config_value::ConfigValue;
use rekv::path_resolver::resolve_set;
use rekv::protocol::RpcRequest;
use rekv::pubsub_engine::PubSubEngine;
use rekv::storage::Store;
use serde_json::{json, Value};
use tempfile::tempdir;
use tokio::sync::{Barrier, RwLock};

const RECORDS: usize = 10_000;
const RANDOM_GETS: usize = 256;
const RANDOM_SETS: usize = 32;
const RESTORES: usize = 16;
const READERS: usize = 4;
const READS_PER_READER: usize = 128;
const LOAD_SAMPLES: usize = 7;

struct DeterministicRandom(u64);

impl DeterministicRandom {
    fn next_index(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 as usize) % limit
    }
}

fn request(id: u64, method: &str, path: String, value: Option<Value>) -> RpcRequest {
    RpcRequest {
        id,
        method: method.to_string(),
        path: Some(path),
        value: value.map(serde_json::from_value).transpose().unwrap(),
    }
}

fn device_path(index: usize) -> String {
    format!(
        "/platform_manager/clusters[id = cluster-0]/racks[id = rack-0]/slots[id = slot-0]/devices[id = device-{index:05}]/baud_rate"
    )
}

fn make_dataset() -> Value {
    let devices: Vec<Value> = (0..RECORDS)
        .map(|index| {
            json!({
                "id": format!("device-{index:05}"),
                "baud_rate": 115200 + (index % 4) * 9600,
                "enabled": index % 2 == 0,
                "protocol": if index % 2 == 0 { "uart" } else { "rs485" },
                "label": format!("sensor-{index:05}")
            })
        })
        .collect();

    json!({
        "platform_manager": {
            "clusters": [{
                "id": "cluster-0",
                "name": "north-wing",
                "enabled": true,
                "racks": [{
                    "id": "rack-0",
                    "name": "rack-alpha",
                    "online": true,
                    "slots": [{
                        "id": "slot-0",
                        "name": "slot-main",
                        "active": true,
                        "devices": devices
                    }]
                }]
            }]
        }
    })
}

fn response_integer(response: &rekv::protocol::RpcResponse) -> i64 {
    let typed: ConfigValue = serde_json::from_value(response.result.clone().unwrap()).unwrap();
    typed.into_json().unwrap().as_i64().unwrap()
}

fn print_percentiles(label: &str, samples: &mut [Duration]) {
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2].as_secs_f64() * 1_000_000.0;
    let p95_index = ((samples.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    let p95 = samples[p95_index].as_secs_f64() * 1_000_000.0;
    println!("{label}: n={} p50={p50:.2}us p95={p95:.2}us", samples.len());
}

fn tree_depth(value: &Value, current: usize) -> usize {
    match value {
        Value::Object(map) => map
            .values()
            .map(|child| tree_depth(child, current + 1))
            .max()
            .unwrap_or(current),
        Value::Array(items) => items
            .iter()
            .map(|child| tree_depth(child, current + 1))
            .max()
            .unwrap_or(current),
        _ => current,
    }
}

fn percentile(samples: &mut [Duration], percentile: f64) -> f64 {
    samples.sort_unstable();
    let index = ((samples.len() as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len() - 1);
    samples[index].as_secs_f64() * 1_000_000.0
}

async fn load_store_samples(
    settings_path: &std::path::Path,
) -> Result<Vec<Duration>, Box<dyn std::error::Error>> {
    let mut samples = Vec::with_capacity(LOAD_SAMPLES);
    for _ in 0..LOAD_SAMPLES {
        let start = Instant::now();
        let store = Store::load_from_file(settings_path)?;
        std::hint::black_box(store.leaf_count());
        samples.push(start.elapsed());
    }
    Ok(samples)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = make_dataset();
    let maximum_depth = tree_depth(&base, 0);
    assert_eq!(
        maximum_depth, 10,
        "generated test tree should have depth 10"
    );
    let encoded_base = serde_json::to_vec(&base)?;

    let directory = tempdir()?;
    let settings_path = directory.path().join("settings.json");
    let delta_path = directory.path().join("_delta.json");
    let write_start = Instant::now();
    let mut base_file = File::create(&settings_path)?;
    base_file.write_all(&encoded_base)?;
    base_file.sync_all()?;
    let base_write_time = write_start.elapsed();

    let mut file_read_samples = Vec::with_capacity(LOAD_SAMPLES);
    for _ in 0..LOAD_SAMPLES {
        let start = Instant::now();
        let bytes = fs::read(&settings_path)?;
        std::hint::black_box(bytes.len());
        file_read_samples.push(start.elapsed());
    }
    println!(
        "dataset: records={RECORDS}, depth={maximum_depth}, bytes={}, base-write+fsync={:.2}ms",
        encoded_base.len(),
        base_write_time.as_secs_f64() * 1_000.0
    );
    println!(
        "base file read only: n={} p50={:.2}us p95={:.2}us",
        file_read_samples.len(),
        percentile(&mut file_read_samples, 0.50),
        percentile(&mut file_read_samples, 0.95)
    );
    let mut base_load_samples = load_store_samples(&settings_path).await?;
    println!(
        "base load (read + JSON + indexes): n={} p50={:.2}ms p95={:.2}ms",
        base_load_samples.len(),
        percentile(&mut base_load_samples, 0.50) / 1_000.0,
        percentile(&mut base_load_samples, 0.95) / 1_000.0
    );

    let base_store = Store::load_from_file(&settings_path)?;
    let in_memory_store = Store::from_value(base.clone(), None);
    let mut random = DeterministicRandom(0x7265_6b76_3130_6b31);
    let get_indices: Vec<_> = (0..RANDOM_GETS)
        .map(|_| random.next_index(RECORDS))
        .collect();

    let mut clone_samples = Vec::with_capacity(RANDOM_SETS);
    for _ in 0..RANDOM_SETS {
        let start = Instant::now();
        std::hint::black_box(in_memory_store.clone());
        clone_samples.push(start.elapsed());
    }
    print_percentiles("in-memory Store clone", &mut clone_samples);

    let mut pure_set_samples = Vec::with_capacity(RANDOM_SETS);
    for operation in 0..RANDOM_SETS {
        let index = random.next_index(RECORDS);
        let mut candidate = in_memory_store.clone();
        let start = Instant::now();
        resolve_set(
            &device_path(index),
            json!(900_000 + operation),
            &mut candidate,
        )?;
        pure_set_samples.push(start.elapsed());
    }
    print_percentiles(
        "resolver selector SET + index rebuild (clone excluded)",
        &mut pure_set_samples,
    );

    let base_pubsub = Arc::new(PubSubEngine::default());
    let base_service =
        ConfigService::new(Arc::new(RwLock::new(base_store)), Arc::clone(&base_pubsub));
    let mut base_get_samples = Vec::with_capacity(get_indices.len());
    for (operation, index) in get_indices.iter().copied().enumerate() {
        let start = Instant::now();
        let response = base_service
            .dispatch(request(
                operation as u64 + 1,
                "get",
                device_path(index),
                None,
            ))
            .await;
        base_get_samples.push(start.elapsed());
        assert!(response.ok, "base GET failed: {:?}", response.error);
    }
    print_percentiles("base random depth-10 id GET", &mut base_get_samples);

    let mut set_samples = Vec::with_capacity(RANDOM_SETS);
    let mut notification_samples = Vec::with_capacity(RANDOM_SETS);
    let mut notification_handoff_samples = Vec::with_capacity(RANDOM_SETS);
    let mut changed_indices = BTreeSet::new();
    for operation in 0..RANDOM_SETS {
        let index = random.next_index(RECORDS);
        let path = device_path(index);
        let mut receiver = base_pubsub.subscribe("/platform_manager");
        let started = Instant::now();
        let notified = tokio::spawn(async move {
            receiver
                .recv()
                .await
                .map(|event| (started.elapsed(), event))
        });
        let response = base_service
            .dispatch(request(
                (RANDOM_GETS + operation + 1) as u64,
                "set",
                path,
                Some(json!({"type":"integer", "value":(900_000 + operation) as i64})),
            ))
            .await;
        let service_time = started.elapsed();
        let (notification_time, event) = notified.await??;
        assert!(response.ok, "delta SET failed: {:?}", response.error);
        assert!(event.path.contains("/devices/"));
        set_samples.push(service_time);
        notification_samples.push(notification_time);
        notification_handoff_samples.push(notification_time.saturating_sub(service_time));
        changed_indices.insert(index);
    }
    print_percentiles("random SET including delta write+fsync", &mut set_samples);
    print_percentiles("SET start to watcher receive", &mut notification_samples);
    print_percentiles(
        "service response to watcher handoff",
        &mut notification_handoff_samples,
    );

    let delta_file_bytes = fs::metadata(&delta_path)?.len();
    let delta_json: Value = serde_json::from_slice(&fs::read(&delta_path)?)?;
    assert_eq!(delta_json.as_object().unwrap().len(), changed_indices.len());
    println!(
        "delta overlay: changed={} unique leaves, file={} bytes",
        changed_indices.len(),
        delta_file_bytes
    );

    let mut overlay_load_samples = load_store_samples(&settings_path).await?;
    println!(
        "overlay load (base + delta + indexes): n={} p50={:.2}ms p95={:.2}ms",
        overlay_load_samples.len(),
        percentile(&mut overlay_load_samples, 0.50) / 1_000.0,
        percentile(&mut overlay_load_samples, 0.95) / 1_000.0
    );
    let overlay_store = Store::load_from_file(&settings_path)?;
    let overlay_service = ConfigService::new(
        Arc::new(RwLock::new(overlay_store)),
        Arc::new(PubSubEngine::default()),
    );
    let mut overlay_get_samples = Vec::with_capacity(get_indices.len());
    for (operation, index) in get_indices.iter().copied().enumerate() {
        let start = Instant::now();
        let response = overlay_service
            .dispatch(request(
                10_000 + operation as u64,
                "get",
                device_path(index),
                None,
            ))
            .await;
        overlay_get_samples.push(start.elapsed());
        assert!(response.ok, "overlay GET failed: {:?}", response.error);
    }
    print_percentiles("overlay random depth-10 id GET", &mut overlay_get_samples);

    let restore_indices: Vec<_> = changed_indices.iter().copied().take(RESTORES).collect();
    let mut restore_samples = Vec::with_capacity(restore_indices.len());
    for (operation, index) in restore_indices.into_iter().enumerate() {
        let start = Instant::now();
        let response = base_service
            .dispatch(request(
                20_000 + operation as u64,
                "restore",
                device_path(index),
                None,
            ))
            .await;
        restore_samples.push(start.elapsed());
        assert!(response.ok, "path restore failed: {:?}", response.error);
        let value = base_service
            .dispatch(request(
                30_000 + index as u64,
                "get",
                device_path(index),
                None,
            ))
            .await;
        assert!(value.ok);
    }
    print_percentiles(
        "path restore including delta rewrite+fsync",
        &mut restore_samples,
    );

    let atomic_index = RECORDS - 1;
    let atomic_baseline = 115_200 + ((atomic_index % 4) * 9_600) as i64;
    let atomic_path = device_path(atomic_index);
    let changed = base_service
        .dispatch(request(
            40_001,
            "set",
            atomic_path.clone(),
            Some(json!({"type":"integer", "value":888_888})),
        ))
        .await;
    assert!(changed.ok);
    let barrier = Arc::new(Barrier::new(READERS + 1));
    let mut readers = Vec::with_capacity(READERS);
    for reader_id in 0..READERS {
        let service = base_service.clone();
        let barrier = Arc::clone(&barrier);
        let path = atomic_path.clone();
        readers.push(tokio::spawn(async move {
            barrier.wait().await;
            for sample in 0..READS_PER_READER {
                let response = service
                    .dispatch(request(
                        50_000 + (reader_id * READS_PER_READER + sample) as u64,
                        "get",
                        path.clone(),
                        None,
                    ))
                    .await;
                assert!(response.ok);
                let value = response_integer(&response);
                assert!(
                    value == atomic_baseline || value == 888_888,
                    "torn value: {value}"
                );
            }
            READS_PER_READER
        }));
    }
    barrier.wait().await;
    let restore_start = Instant::now();
    let restored = base_service
        .dispatch(request(60_001, "restore", atomic_path.clone(), None))
        .await;
    let restore_latency = restore_start.elapsed();
    assert!(restored.ok);
    let reads = futures_count(readers).await?;
    let final_value = base_service
        .dispatch(request(60_002, "get", atomic_path, None))
        .await;
    assert_eq!(response_integer(&final_value), atomic_baseline);
    println!("read atomicity: {reads} concurrent reads saw baseline or override only");
    println!(
        "restore while readers active: {:.2}us",
        restore_latency.as_secs_f64() * 1_000.0
    );

    assert_eq!(fs::read(&settings_path)?, encoded_base);
    println!("original settings file unchanged after delta operations");
    Ok(())
}

async fn futures_count(
    readers: Vec<tokio::task::JoinHandle<usize>>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut total = 0;
    for reader in readers {
        total += reader.await?;
    }
    Ok(total)
}
