import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import type { BenchmarkResult } from "../stats.js";

interface RustBenchJsonItem {
  surface: string;
  codec: string;
  durability: string;
  total_mutations: number;
  batch_size: number;
  total_duration_ms: f64;
  throughput_ops_sec: f64;
  p50_us: f64;
  p95_us: f64;
  p99_us: f64;
  mean_us: f64;
  min_us: f64;
  max_us: f64;
  stddev_us: f64;
  disk_bytes: number;
}
type f64 = number;

export async function runRustBenchmark(
  workload: number,
  batchSize: number,
  codec: "clear" | "aes256" | "both" = "both",
  durability: string = "durable"
): Promise<BenchmarkResult[]> {
  const binaryCandidates = [
    resolve(process.cwd(), "../../target/release/lilia_bench"),
    resolve(process.cwd(), "../../target/release/lilia_bench.exe"),
    resolve(process.cwd(), "target/release/lilia_bench"),
    resolve(process.cwd(), "target/release/lilia_bench.exe"),
  ];
  const binary = binaryCandidates.find(existsSync);
  if (!binary) {
    throw new Error(
      `Rust benchmark binary 'lilia_bench' not found. Run 'cargo build --release -p lilia-storage-sqlite --bin lilia_bench' first.`
    );
  }

  const args = [
    "--count",
    String(workload),
    "--batch-size",
    String(batchSize),
    "--codec",
    codec,
    "--durability",
    durability,
    "--json",
  ];

  const stdout = await new Promise<string>((resolvePromise, reject) => {
    const child = spawn(binary, args, { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (chunk: Buffer) => {
      out += chunk.toString("utf8");
    });
    child.stderr.on("data", (chunk: Buffer) => {
      err += chunk.toString("utf8");
    });
    child.on("close", (code) => {
      if (code === 0) {
        resolvePromise(out);
      } else {
        reject(
          new Error(
            `Rust benchmark failed with exit code ${code}: ${err.trim()}`
          )
        );
      }
    });
    child.on("error", reject);
  });

  const parsed = JSON.parse(stdout) as RustBenchJsonItem[];
  return parsed.map((item) => ({
    surface: "Rust Native Embedded",
    codec: item.codec === "aes-256-gcm" ? "aes-256-gcm" : "cleartext",
    workload: item.total_mutations,
    batchSize: item.batch_size,
    totalDurationMs: item.total_duration_ms,
    throughputOpsSec: item.throughput_ops_sec,
    p50Us: item.p50_us,
    p95Us: item.p95_us,
    p99Us: item.p99_us,
    meanUs: item.mean_us,
    minUs: item.min_us,
    maxUs: item.max_us,
    stddevUs: item.stddev_us,
    diskBytes: item.disk_bytes,
  }));
}
