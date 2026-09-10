use criterion::{criterion_group, criterion_main, Criterion};
use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use std::hint::black_box;

fn kv_write(c: &mut Criterion) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = Database::open(DatabaseOptions::durable(
        directory.path().join("bench.lilia"),
    ))
    .expect("database");
    let mut sequence = 0_u64;
    c.bench_function("kv_set_1k", |benchmark| {
        benchmark.iter(|| {
            sequence += 1;
            database
                .batch(&[BatchOperation::KvSet {
                    namespace: "bench".into(),
                    key: sequence.to_be_bytes().to_vec(),
                    value: vec![42; 1_024],
                    if_version: None,
                    expires_at_ms: None,
                }])
                .expect("write");
            black_box(sequence);
        });
    });
}

criterion_group!(benches, kv_write);
criterion_main!(benches);
