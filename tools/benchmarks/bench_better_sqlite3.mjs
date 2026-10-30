import Database from "better-sqlite3";
import { mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

function percentile(sorted, p) {
  if (sorted.length === 0) return 0;
  if (sorted.length === 1) return sorted[0];
  const rank = (p / 100) * (sorted.length - 1);
  const lower = Math.floor(rank);
  const upper = Math.ceil(rank);
  const weight = rank - lower;
  return sorted[lower] * (1 - weight) + sorted[upper] * weight;
}

function parseArgs() {
  const args = process.argv.slice(2);
  let count = 10000;
  let batchSize = 1;
  let durability = "durable";
  let json = false;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--count" && args[i + 1]) count = parseInt(args[++i], 10);
    else if (args[i] === "--batch-size" && args[i + 1]) batchSize = parseInt(args[++i], 10);
    else if (args[i] === "--durability" && args[i + 1]) durability = args[++i].toLowerCase();
    else if (args[i] === "--json") json = true;
  }
  return { count, batchSize, durability, json };
}

function main() {
  const { count, batchSize, durability, json } = parseArgs();
  const synchronous = durability === "balanced" ? "NORMAL" : durability === "performance" ? "OFF" : "FULL";

  const dir = mkdtempSync(join(tmpdir(), "bench-bs3-"));
  const dbPath = join(dir, "bench.db");

  try {
    const db = new Database(dbPath);
    db.pragma("journal_mode = WAL");
    db.pragma(`synchronous = ${synchronous}`);
    db.pragma("temp_store = MEMORY");
    db.pragma("cache_size = -65536");

    db.exec(`
      CREATE TABLE IF NOT EXISTS _lilia_kv(
        namespace TEXT NOT NULL,
        key BLOB NOT NULL,
        value BLOB NOT NULL,
        version INTEGER NOT NULL CHECK(version > 0),
        expires_at_ms INTEGER,
        PRIMARY KEY(namespace, key)
      ) STRICT, WITHOUT ROWID;
    `);

    const insertStmt = db.prepare(`
      INSERT INTO _lilia_kv(namespace, key, value, version, expires_at_ms)
      VALUES (?, ?, ?, ?, ?)
      ON CONFLICT(namespace, key) DO UPDATE SET
        value = excluded.value,
        version = excluded.version,
        expires_at_ms = excluded.expires_at_ms
    `);

    const payload = Buffer.alloc(256, 42);

    // Warmup (300 mutations)
    const warmupBatches = Math.ceil(300 / Math.max(1, batchSize));
    let wGen = 0;
    const runWarmupTx = db.transaction((chunkSize, bIdx) => {
      for (let i = 0; i < chunkSize; i++) {
        const id = bIdx * batchSize + i;
        insertStmt.run("warmup", Buffer.from(`k_${id}`), payload, 1, null);
      }
    });

    for (let b = 0; b < warmupBatches; b++) {
      const cur = Math.min(300 - wGen, batchSize);
      runWarmupTx(cur, b);
      wGen += cur;
    }

    // Benchmark
    const numBatches = Math.ceil(count / Math.max(1, batchSize));
    const latenciesUs = new Array(numBatches);

    const runBatchTx = db.transaction((chunkSize, bIdx) => {
      for (let i = 0; i < chunkSize; i++) {
        const id = bIdx * batchSize + i;
        const keyBuf = Buffer.from(`bench_key_${String(id).padStart(8, "0")}`);
        const valBuf = Buffer.alloc(256, id % 256);
        insertStmt.run("bench", keyBuf, valBuf, 1, null);
      }
    });

    let generated = 0;
    const startAll = process.hrtime.bigint();

    for (let b = 0; b < numBatches; b++) {
      const cur = Math.min(count - generated, batchSize);
      const bStart = process.hrtime.bigint();
      runBatchTx(cur, b);
      const bEnd = process.hrtime.bigint();
      const elapsedNs = Number(bEnd - bStart);
      latenciesUs[b] = elapsedNs / (cur * 1000);
      generated += cur;
    }

    const endAll = process.hrtime.bigint();
    const totalDurationMs = Number(endAll - startAll) / 1_000_000;
    const throughputOpsSec = (count / totalDurationMs) * 1000;

    latenciesUs.sort((a, b) => a - b);
    const p50Us = percentile(latenciesUs, 50);
    const p95Us = percentile(latenciesUs, 95);
    const p99Us = percentile(latenciesUs, 99);
    const minUs = latenciesUs[0] || 0;
    const maxUs = latenciesUs[latenciesUs.length - 1] || 0;
    const sum = latenciesUs.reduce((acc, v) => acc + v, 0);
    const meanUs = sum / latenciesUs.length;
    const variance = latenciesUs.reduce((acc, v) => acc + Math.pow(v - meanUs, 2), 0) / latenciesUs.length;
    const stddevUs = Math.sqrt(variance);

    db.close();
    let diskBytes = 0;
    try {
      diskBytes = statSync(dbPath).size;
    } catch {}

    const result = {
      surface: "Node.js (better-sqlite3)",
      codec: "cleartext",
      durability,
      total_mutations: count,
      batch_size: batchSize,
      total_duration_ms: totalDurationMs,
      throughput_ops_sec: throughputOpsSec,
      p50_us: p50Us,
      p95_us: p95Us,
      p99_us: p99Us,
      mean_us: meanUs,
      min_us: minUs,
      max_us: maxUs,
      stddev_us: stddevUs,
      disk_bytes: diskBytes,
    };

    if (json) {
      console.log(JSON.stringify(result));
    } else {
      console.log(`[better-sqlite3] ${Math.round(throughputOpsSec)} ops/sec (p50: ${p50Us.toFixed(2)} µs, p95: ${p95Us.toFixed(2)} µs)`);
    }
  } finally {
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch {}
  }
}

main();
