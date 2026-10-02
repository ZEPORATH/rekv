use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rekv::path_parser::QueryPath;
use rekv::path_resolver::{resolve_get, resolve_paths};
use rekv::storage::Store;
use serde_json::{json, Value};

fn config_operations(c: &mut Criterion) {
    let settings_json = std::fs::read_to_string("tests/fixtures/settings.json").unwrap();
    let settings_value: Value = serde_json::from_str(&settings_json).unwrap();
    let store = Store::from_value(settings_value, None);
    let query_text = "/platform_manager/io_devices[id = ECU0]/baud_rate";
    let query = QueryPath::parse(query_text).unwrap();

    c.bench_function("json_decode", |bench| {
        bench.iter(|| serde_json::from_str::<Value>(black_box(&settings_json)).unwrap())
    });
    c.bench_function("json_encode", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(store.tree())).unwrap())
    });
    c.bench_function("get_lookup", |bench| {
        bench.iter(|| resolve_get(black_box(query_text), black_box(&store)).unwrap())
    });
    c.bench_function("set_lookup", |bench| {
        bench.iter(|| resolve_paths(black_box(&query), black_box(&store)))
    });
    c.bench_function("path_resolution", |bench| {
        bench.iter(|| QueryPath::parse(black_box(query_text)).unwrap())
    });
    c.bench_function("set_operation_without_transport", |bench| {
        bench.iter_batched(
            || store.clone(),
            |mut candidate| {
                rekv::path_resolver::resolve_set(
                    black_box(query_text),
                    json!(52.0),
                    black_box(&mut candidate),
                )
                .unwrap()
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, config_operations);
criterion_main!(benches);
