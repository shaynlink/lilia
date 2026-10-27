import os from "node:os";
import { resolve } from "node:path";
import { writeFile, mkdir } from "node:fs/promises";
import { dirname } from "node:path";
import { runRustBenchmark } from "./drivers/rust_embedded.js";
import { runNodeEmbeddedBenchmark } from "./drivers/node_embedded.js";
import { runDaemonIpcBenchmark } from "./drivers/daemon_ipc.js";
import { runDaemonHttpBenchmark } from "./drivers/daemon_http.js";
import {
  printConsoleTable,
  printOverheadTable,
  generateMarkdown,
  exportJson,
} from "./reporter.js";
import type { BenchmarkResult } from "./stats.js";

interface CliOptions {
  workload: "10k" | "100k" | "all";
  surfaces: string[];
  codecs: ("cleartext" | "aes-256-gcm")[];
  markdownPath?: string;
  jsonPath?: string;
}

function parseArgs(): CliOptions {
  const args = process.argv.slice(2);
  let workload: "10k" | "100k" | "all" = "10k";
  let surfaces = ["rust", "node", "ipc", "http"];
  let codecs: ("cleartext" | "aes-256-gcm")[] = ["cleartext", "aes-256-gcm"];
  let markdownPath = resolve(process.cwd(), "../../docs/benchmarks.md");
  let jsonPath = resolve(process.cwd(), "../../target/benchmark-results.json");

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--workload" && args[i + 1]) {
      const val = args[++i]?.toLowerCase();
      if (val === "100k" || val === "all") workload = val;
      else workload = "10k";
    } else if (arg === "--surfaces" && args[i + 1]) {
      const val = args[++i]!;
      if (val !== "all") {
        surfaces = val.split(",").map((s) => s.trim().toLowerCase());
      }
    } else if (arg === "--codec" && args[i + 1]) {
      const val = args[++i]?.toLowerCase();
      if (val === "clear") codecs = ["cleartext"];
      else if (val === "aes" || val === "aes256") codecs = ["aes-256-gcm"];
    } else if (arg === "--markdown" && args[i + 1]) {
      markdownPath = resolve(process.cwd(), args[++i]!);
    } else if (arg === "--json" && args[i + 1]) {
      jsonPath = resolve(process.cwd(), args[++i]!);
    }
  }

  return { workload, surfaces, codecs, markdownPath, jsonPath };
}

async function main(): Promise<void> {
  const options = parseArgs();

  console.log("\n🚀 Initializing LiliaDB Calibrated Benchmark Harness...");
  console.log(`   Workload: ${options.workload}`);
  console.log(`   Surfaces: ${options.surfaces.join(", ")}`);
  console.log(`   Codecs:   ${options.codecs.join(", ")}`);

  const workloadsToRun: Array<{ count: number; batchSize: number; desc: string }> = [];

  if (options.workload === "10k" || options.workload === "all") {
    // 10k single-mutation transactions: measures true per-op commit latency
    workloadsToRun.push({
      count: 10_000,
      batchSize: 1,
      desc: "10,000 mutations (single-op transactions, batch=1)",
    });
    // 10k batched ingestion: measures batched ingest throughput
    workloadsToRun.push({
      count: 10_000,
      batchSize: 100,
      desc: "10,000 mutations (batched ingestion, batch=100)",
    });
  }

  if (options.workload === "100k" || options.workload === "all") {
    // 100k bulk ingestion: measures heavy sustained throughput
    workloadsToRun.push({
      count: 100_000,
      batchSize: 500,
      desc: "100,000 mutations (high-throughput bulk ingest, batch=500)",
    });
  }

  const allResults: BenchmarkResult[] = [];

  for (const w of workloadsToRun) {
    console.log(`\n▶ Starting scenario: ${w.desc}`);

    // 1. Rust Native Embedded
    if (options.surfaces.includes("rust")) {
      for (const codec of options.codecs) {
        process.stdout.write(
          `  - [Rust Native Embedded] running ${w.count} (${codec})... `
        );
        try {
          const rustCodecs = codec === "aes-256-gcm" ? "aes256" : "clear";
          const res = await runRustBenchmark(w.count, w.batchSize, rustCodecs);
          allResults.push(...res);
          const first = res[0];
          console.log(
            `DONE: ${Math.round(first?.throughputOpsSec ?? 0)} op/s (p50: ${first?.p50Us.toFixed(2)} µs)`
          );
        } catch (err) {
          console.log(`ERROR: ${(err as Error).message}`);
        }
      }
    }

    // 2. Node-API Embedded
    if (options.surfaces.includes("node")) {
      for (const codec of options.codecs) {
        process.stdout.write(
          `  - [Node-API Embedded] running ${w.count} (${codec})... `
        );
        try {
          const res = await runNodeEmbeddedBenchmark(w.count, w.batchSize, codec);
          allResults.push(res);
          console.log(
            `DONE: ${Math.round(res.throughputOpsSec)} op/s (p50: ${res.p50Us.toFixed(2)} µs)`
          );
        } catch (err) {
          console.log(`ERROR: ${(err as Error).message}`);
        }
      }
    }

    // 3. Daemon IPC (Unix Socket / Pipe)
    if (options.surfaces.includes("ipc")) {
      for (const codec of options.codecs) {
        process.stdout.write(
          `  - [Daemon IPC Socket] running ${w.count} (${codec})... `
        );
        try {
          const res = await runDaemonIpcBenchmark(w.count, w.batchSize, codec);
          allResults.push(res);
          console.log(
            `DONE: ${Math.round(res.throughputOpsSec)} op/s (p50: ${res.p50Us.toFixed(2)} µs)`
          );
        } catch (err) {
          console.log(`ERROR: ${(err as Error).message}`);
        }
      }
    }

    // 4. Daemon HTTP (Loopback REST Gateway)
    if (options.surfaces.includes("http")) {
      for (const codec of options.codecs) {
        process.stdout.write(
          `  - [Daemon HTTP Gateway] running ${w.count} (${codec})... `
        );
        try {
          const res = await runDaemonHttpBenchmark(w.count, w.batchSize, codec);
          allResults.push(res);
          console.log(
            `DONE: ${Math.round(res.throughputOpsSec)} op/s (p50: ${res.p50Us.toFixed(2)} µs)`
          );
        } catch (err) {
          console.log(`ERROR: ${(err as Error).message}`);
        }
      }
    }
  }

  // Print results
  printConsoleTable(allResults);
  printOverheadTable(allResults);

  // Generate outputs
  const cpus = os.cpus();
  const envInfo = {
    os: `${os.type()} ${os.release()}`,
    arch: os.arch(),
    cpu: cpus.length > 0 ? `${cpus[0]?.model} (${cpus.length} cores)` : "Unknown",
    nodeVersion: process.version,
    date: new Date().toISOString().split("T")[0]!,
  };

  if (options.markdownPath) {
    const md = generateMarkdown(allResults, envInfo);
    await mkdir(dirname(options.markdownPath), { recursive: true });
    await writeFile(options.markdownPath, md, "utf8");
    console.log(`📄 Official markdown benchmark report written to: ${options.markdownPath}`);
  }

  if (options.jsonPath) {
    await exportJson(allResults, options.jsonPath);
    console.log(`📊 Machine-readable benchmark results exported to: ${options.jsonPath}\n`);
  }
}

main().catch((err) => {
  console.error("Benchmark runner failed:", err);
  process.exit(1);
});
