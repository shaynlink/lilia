#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use lilia_core::{Aes256GcmCodec, BatchOperation, Durability, IdentityCodec, ValueCodec};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(
    name = "lilia-bench",
    about = "Calibrated benchmark runner for LiliaDB native Rust engine"
)]
struct Arguments {
    /// Total number of mutations to execute
    #[arg(long, default_value_t = 10_000)]
    count: usize,

    /// Number of mutations per batch (1 = single-op transactions)
    #[arg(long, default_value_t = 1)]
    batch_size: usize,

    /// Codec to benchmark: "clear", "aes256", or "both"
    #[arg(long, default_value = "both")]
    codec: String,

    /// Durability profile: "durable", "balanced", or "performance"
    #[arg(long, default_value = "durable")]
    durability: String,

    /// Include raw `SQLite` native (C / rusqlite) baseline
    #[arg(long)]
    raw_sqlite: bool,

    /// Output results as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchMetrics {
    pub surface: String,
    pub codec: String,
    pub durability: String,
    pub total_mutations: usize,
    pub batch_size: usize,
    pub total_duration_ms: f64,
    pub throughput_ops_sec: f64,
    pub p50_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub mean_us: f64,
    pub min_us: f64,
    pub max_us: f64,
    pub stddev_us: f64,
    pub disk_bytes: u64,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let weight = rank - lower as f64;
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}

fn run_single_benchmark(
    codec_name: &str,
    codec: Arc<dyn ValueCodec>,
    durability_name: &str,
    durability: Durability,
    count: usize,
    batch_size: usize,
) -> Result<BenchMetrics, Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let db_path: PathBuf = temp_dir.path().join("bench.lilia");

    let mut options = DatabaseOptions::durable(&db_path).with_codec(codec);
    options.durability = durability;
    let db = Database::open(options)?;

    // 1. Warmup phase (300 mutations)
    let warmup_ops: Vec<BatchOperation> = (0..300)
        .map(|i| BatchOperation::KvSet {
            namespace: "warmup".into(),
            key: format!("k_{i}").into_bytes(),
            value: vec![42u8; 256],
            if_version: None,
            expires_at_ms: None,
        })
        .collect();
    for chunk in warmup_ops.chunks(batch_size.max(1)) {
        db.batch(chunk)?;
    }

    // 2. Timed benchmark phase
    let num_batches = count.div_ceil(batch_size);
    let mut batch_latencies_us = Vec::with_capacity(num_batches);

    let start_all = Instant::now();

    let mut generated = 0;
    for b in 0..num_batches {
        let current_batch_size = (count - generated).min(batch_size);
        let ops: Vec<BatchOperation> = (0..current_batch_size)
            .map(|i| {
                let id = b * batch_size + i;
                BatchOperation::KvSet {
                    namespace: "bench".into(),
                    key: format!("bench_key_{id:08}").into_bytes(),
                    value: vec![(id % 256) as u8; 256],
                    if_version: None,
                    expires_at_ms: None,
                }
            })
            .collect();

        let batch_start = Instant::now();
        db.batch(&ops)?;
        let elapsed = batch_start.elapsed();

        let op_latency_us = (elapsed.as_nanos() as f64) / (current_batch_size as f64 * 1_000.0);
        batch_latencies_us.push(op_latency_us);
        generated += current_batch_size;
    }

    let total_duration = start_all.elapsed();
    let total_duration_ms = total_duration.as_secs_f64() * 1_000.0;
    let throughput_ops_sec = (count as f64) / total_duration.as_secs_f64();

    // Calculate percentiles
    batch_latencies_us.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50_us = percentile(&batch_latencies_us, 50.0);
    let p95_us = percentile(&batch_latencies_us, 95.0);
    let p99_us = percentile(&batch_latencies_us, 99.0);
    let min_us = *batch_latencies_us.first().unwrap_or(&0.0);
    let max_us = *batch_latencies_us.last().unwrap_or(&0.0);

    let sum: f64 = batch_latencies_us.iter().sum();
    let mean_us = sum / batch_latencies_us.len() as f64;
    let variance: f64 = batch_latencies_us
        .iter()
        .map(|v| (v - mean_us).powi(2))
        .sum::<f64>()
        / batch_latencies_us.len() as f64;
    let stddev_us = variance.sqrt();

    // Check disk file size
    drop(db);
    let disk_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    Ok(BenchMetrics {
        surface: "Rust Native Embedded".into(),
        codec: codec_name.into(),
        durability: durability_name.into(),
        total_mutations: count,
        batch_size,
        total_duration_ms,
        throughput_ops_sec,
        p50_us,
        p95_us,
        p99_us,
        mean_us,
        min_us,
        max_us,
        stddev_us,
        disk_bytes,
    })
}

fn run_raw_sqlite_benchmark(
    durability_name: &str,
    synchronous: &str,
    count: usize,
    batch_size: usize,
) -> Result<BenchMetrics, Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let db_path: PathBuf = temp_dir.path().join("bench_raw.db");

    let mut conn = rusqlite::Connection::open(&db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", synchronous)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -65536)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _lilia_kv(
           namespace TEXT NOT NULL,
           key BLOB NOT NULL,
           value BLOB NOT NULL,
           version INTEGER NOT NULL CHECK(version > 0),
           expires_at_ms INTEGER,
           PRIMARY KEY(namespace,key)
         ) STRICT, WITHOUT ROWID;",
    )?;

    // 1. Warmup phase (300 mutations)
    let payload = vec![42u8; 256];
    let num_warmup_batches = 300usize.div_ceil(batch_size.max(1));
    let mut w_generated = 0;
    for b in 0..num_warmup_batches {
        let chunk_size = (300 - w_generated).min(batch_size);
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO _lilia_kv(namespace,key,value,version,expires_at_ms) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,version=excluded.version,expires_at_ms=excluded.expires_at_ms",
            )?;
            for i in 0..chunk_size {
                let id = b * batch_size + i;
                let key = format!("k_{id}").into_bytes();
                stmt.execute(rusqlite::params!["warmup", key, payload, 1i64, None::<i64>])?;
            }
        }
        tx.commit()?;
        w_generated += chunk_size;
    }

    // 2. Timed benchmark phase
    let num_batches = count.div_ceil(batch_size.max(1));
    let mut batch_latencies_us = Vec::with_capacity(num_batches);

    let start_all = Instant::now();
    let mut generated = 0;

    for b in 0..num_batches {
        let current_batch_size = (count - generated).min(batch_size);
        let batch_start = Instant::now();

        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO _lilia_kv(namespace,key,value,version,expires_at_ms) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,version=excluded.version,expires_at_ms=excluded.expires_at_ms",
            )?;
            for i in 0..current_batch_size {
                let id = b * batch_size + i;
                let key = format!("bench_key_{id:08}").into_bytes();
                let val = vec![(id % 256) as u8; 256];
                stmt.execute(rusqlite::params!["bench", key, val, 1i64, None::<i64>])?;
            }
        }
        tx.commit()?;

        let elapsed = batch_start.elapsed();
        let op_latency_us = (elapsed.as_nanos() as f64) / (current_batch_size as f64 * 1_000.0);
        batch_latencies_us.push(op_latency_us);
        generated += current_batch_size;
    }

    let total_duration = start_all.elapsed();
    let total_duration_ms = total_duration.as_secs_f64() * 1_000.0;
    let throughput_ops_sec = (count as f64) / total_duration.as_secs_f64();

    batch_latencies_us.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50_us = percentile(&batch_latencies_us, 50.0);
    let p95_us = percentile(&batch_latencies_us, 95.0);
    let p99_us = percentile(&batch_latencies_us, 99.0);
    let min_us = *batch_latencies_us.first().unwrap_or(&0.0);
    let max_us = *batch_latencies_us.last().unwrap_or(&0.0);

    let sum: f64 = batch_latencies_us.iter().sum();
    let mean_us = sum / batch_latencies_us.len() as f64;
    let variance: f64 = batch_latencies_us
        .iter()
        .map(|v| (v - mean_us).powi(2))
        .sum::<f64>()
        / batch_latencies_us.len() as f64;
    let stddev_us = variance.sqrt();

    drop(conn);
    let disk_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    Ok(BenchMetrics {
        surface: "SQLite Native (C / Rusqlite)".into(),
        codec: "cleartext".into(),
        durability: durability_name.into(),
        total_mutations: count,
        batch_size,
        total_duration_ms,
        throughput_ops_sec,
        p50_us,
        p95_us,
        p99_us,
        mean_us,
        min_us,
        max_us,
        stddev_us,
        disk_bytes,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Arguments::parse();

    let durability = match args.durability.to_lowercase().as_str() {
        "balanced" => Durability::Balanced,
        "performance" => Durability::Performance,
        _ => Durability::Durable,
    };

    let synchronous = match durability {
        Durability::Durable => "FULL",
        Durability::Balanced => "NORMAL",
        Durability::Performance => "OFF",
    };

    let run_clear = args.codec == "clear" || args.codec == "both";
    let run_aes = args.codec == "aes256" || args.codec == "both";

    let mut results = Vec::new();

    if args.raw_sqlite {
        let metrics =
            run_raw_sqlite_benchmark(&args.durability, synchronous, args.count, args.batch_size)?;
        results.push(metrics);
    }

    if run_clear {
        let metrics = run_single_benchmark(
            "cleartext",
            Arc::new(IdentityCodec),
            &args.durability,
            durability,
            args.count,
            args.batch_size,
        )?;
        results.push(metrics);
    }

    if run_aes {
        let metrics = run_single_benchmark(
            "aes-256-gcm",
            Arc::new(Aes256GcmCodec::from_passphrase(
                "liliadb-benchmark-official-key",
            )),
            &args.durability,
            durability,
            args.count,
            args.batch_size,
        )?;
        results.push(metrics);
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        println!(
            "\n=== LiliaDB Native Rust Benchmark ({}, count={}, batch={}) ===",
            args.durability, args.count, args.batch_size
        );
        println!(
            "{:<14} | {:<12} | {:>14} | {:>10} | {:>10} | {:>10} | {:>10}",
            "Codec", "Durability", "Throughput", "p50 (µs)", "p95 (µs)", "p99 (µs)", "Disk (KB)"
        );
        println!(
            "{:-<14}-|-{:-<12}-|-{:-<14}-|-{:-<10}-|-{:-<10}-|-{:-<10}-|-{:-<10}",
            "", "", "", "", "", "", ""
        );
        for m in &results {
            println!(
                "{:<14} | {:<12} | {:>10.0} ops/s | {:>10.2} | {:>10.2} | {:>10.2} | {:>10.1}",
                m.codec,
                m.durability,
                m.throughput_ops_sec,
                m.p50_us,
                m.p95_us,
                m.p99_us,
                m.disk_bytes as f64 / 1024.0
            );
        }

        if results.len() == 2 {
            let clear = &results[0];
            let aes = &results[1];
            let throughput_diff = (clear.throughput_ops_sec - aes.throughput_ops_sec)
                / clear.throughput_ops_sec
                * 100.0;
            let p50_diff = (aes.p50_us - clear.p50_us) / clear.p50_us * 100.0;
            let disk_diff =
                (aes.disk_bytes as f64 - clear.disk_bytes as f64) / clear.disk_bytes as f64 * 100.0;

            println!("\n--- AES-256-GCM vs Cleartext Impact ---");
            println!("  Throughput Impact: {:.1}% change", -throughput_diff);
            println!(
                "  Latency p50 Delta: {:+.2} µs ({:+.1}%)",
                aes.p50_us - clear.p50_us,
                p50_diff
            );
            println!("  Disk Expansion:    {disk_diff:+.1}%\n");
        }
    }

    Ok(())
}
