import argparse
import json
import math
import os
import shutil
import sqlite3
import tempfile
import time

def percentile(sorted_list, p):
    if not sorted_list:
        return 0.0
    if len(sorted_list) == 1:
        return float(sorted_list[0])
    rank = (p / 100.0) * (len(sorted_list) - 1)
    lower = int(math.floor(rank))
    upper = int(math.ceil(rank))
    weight = rank - lower
    return float(sorted_list[lower] * (1.0 - weight) + sorted_list[upper] * weight)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=10000)
    parser.add_argument("--batch-size", type=int, default=1)
    parser.add_argument("--durability", type=str, default="durable")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    durability = args.durability.lower()
    synchronous = "NORMAL" if durability == "balanced" else ("OFF" if durability == "performance" else "FULL")

    temp_dir = tempfile.mkdtemp(prefix="bench-pysqlite-")
    db_path = os.path.join(temp_dir, "bench.db")

    try:
        # isolation_level=None enables autocommit so we control transactions explicitly
        conn = sqlite3.connect(db_path, isolation_level=None)
        cursor = conn.cursor()
        cursor.execute("PRAGMA journal_mode = WAL;")
        cursor.execute(f"PRAGMA synchronous = {synchronous};")
        cursor.execute("PRAGMA temp_store = MEMORY;")
        cursor.execute("PRAGMA cache_size = -65536;")

        cursor.execute("""
            CREATE TABLE IF NOT EXISTS _lilia_kv(
                namespace TEXT NOT NULL,
                key BLOB NOT NULL,
                value BLOB NOT NULL,
                version INTEGER NOT NULL CHECK(version > 0),
                expires_at_ms INTEGER,
                PRIMARY KEY(namespace, key)
            ) STRICT, WITHOUT ROWID;
        """)

        insert_sql = """
            INSERT INTO _lilia_kv(namespace, key, value, version, expires_at_ms)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(namespace, key) DO UPDATE SET
                value = excluded.value,
                version = excluded.version,
                expires_at_ms = excluded.expires_at_ms
        """

        payload = bytes([42] * 256)

        # Warmup (300 mutations)
        warmup_batches = (300 + args.batch_size - 1) // max(1, args.batch_size)
        w_gen = 0
        for b in range(warmup_batches):
            cur = min(300 - w_gen, args.batch_size)
            cursor.execute("BEGIN;")
            for i in range(cur):
                idx = b * args.batch_size + i
                cursor.execute(insert_sql, ("warmup", f"k_{idx}".encode("utf-8"), payload, 1, None))
            cursor.execute("COMMIT;")
            w_gen += cur

        # Timed benchmark
        num_batches = (args.count + args.batch_size - 1) // max(1, args.batch_size)
        latencies_us = []

        generated = 0
        start_all = time.perf_counter_ns()

        for b in range(num_batches):
            cur = min(args.count - generated, args.batch_size)
            b_start = time.perf_counter_ns()
            cursor.execute("BEGIN;")
            for i in range(cur):
                idx = b * args.batch_size + i
                key = f"bench_key_{idx:08d}".encode("utf-8")
                val = bytes([idx % 256] * 256)
                cursor.execute(insert_sql, ("bench", key, val, 1, None))
            cursor.execute("COMMIT;")
            b_end = time.perf_counter_ns()
            elapsed_ns = b_end - b_start
            latencies_us.append(elapsed_ns / (cur * 1000.0))
            generated += cur

        end_all = time.perf_counter_ns()
        total_duration_ms = (end_all - start_all) / 1_000_000.0
        throughput_ops_sec = (args.count / total_duration_ms) * 1000.0 if total_duration_ms > 0 else 0.0

        latencies_us.sort()
        p50_us = percentile(latencies_us, 50.0)
        p95_us = percentile(latencies_us, 95.0)
        p99_us = percentile(latencies_us, 99.0)
        min_us = latencies_us[0] if latencies_us else 0.0
        max_us = latencies_us[-1] if latencies_us else 0.0
        mean_us = sum(latencies_us) / len(latencies_us) if latencies_us else 0.0
        variance = sum((v - mean_us) ** 2 for v in latencies_us) / len(latencies_us) if latencies_us else 0.0
        stddev_us = math.sqrt(variance)

        conn.close()
        disk_bytes = 0
        try:
            disk_bytes = os.path.getsize(db_path)
        except OSError:
            pass

        result = {
            "surface": "Python (sqlite3)",
            "codec": "cleartext",
            "durability": durability,
            "total_mutations": args.count,
            "batch_size": args.batch_size,
            "total_duration_ms": total_duration_ms,
            "throughput_ops_sec": throughput_ops_sec,
            "p50_us": p50_us,
            "p95_us": p95_us,
            "p99_us": p99_us,
            "mean_us": mean_us,
            "min_us": min_us,
            "max_us": max_us,
            "stddev_us": stddev_us,
            "disk_bytes": disk_bytes,
        }

        if args.json:
            print(json.dumps(result))
        else:
            print(f"[python sqlite3] {throughput_ops_sec:.0f} ops/sec (p50: {p50_us:.2f} µs, p95: {p95_us:.2f} µs)")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)

if __name__ == "__main__":
    main()
