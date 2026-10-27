import { mkdtemp, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Database, type BatchOperation } from "../../index.js";
import { calculateStats, type BenchmarkResult } from "../stats.js";

export async function runNodeEmbeddedBenchmark(
  workload: number,
  batchSize: number,
  codec: "cleartext" | "aes-256-gcm" = "cleartext"
): Promise<BenchmarkResult> {
  const dir = await mkdtemp(join(tmpdir(), "lilia-node-emb-bench-"));
  const dbPath = join(dir, "bench.lilia");

  const encryptionKey =
    codec === "aes-256-gcm" ? "liliadb-benchmark-official-key" : undefined;

  const db = await Database.open({
    path: dbPath,
    mode: "embedded",
    durability: "durable",
    encryptionKey,
  });

  try {
    const payloadBytes = new Uint8Array(256).fill(42);

    // Warmup phase (200 mutations)
    const warmupOps: BatchOperation[] = Array.from({ length: 200 }, (_, i) => ({
      model: "kv_set",
      namespace: "warmup",
      key: new TextEncoder().encode(`k_${i}`),
      value: payloadBytes,
    }));
    for (let i = 0; i < warmupOps.length; i += batchSize) {
      await db.batch(warmupOps.slice(i, i + batchSize));
    }

    // Timed benchmark phase
    const numBatches = Math.ceil(workload / batchSize);
    const latenciesUs: number[] = new Array<number>(numBatches);

    let generated = 0;
    const startAll = process.hrtime.bigint();

    for (let b = 0; b < numBatches; b++) {
      const currentBatchSize = Math.min(workload - generated, batchSize);
      const ops: BatchOperation[] = new Array<BatchOperation>(currentBatchSize);
      for (let i = 0; i < currentBatchSize; i++) {
        const id = b * batchSize + i;
        ops[i] = {
          model: "kv_set",
          namespace: "bench",
          key: new TextEncoder().encode(`bench_key_${String(id).padStart(8, "0")}`),
          value: payloadBytes,
        };
      }

      const batchStart = process.hrtime.bigint();
      await db.batch(ops);
      const batchEnd = process.hrtime.bigint();

      const elapsedNs = Number(batchEnd - batchStart);
      latenciesUs[b] = elapsedNs / (currentBatchSize * 1_000);
      generated += currentBatchSize;
    }

    const endAll = process.hrtime.bigint();
    const totalDurationMs = Number(endAll - startAll) / 1_000_000;

    await db.close();

    let diskBytes: number | undefined;
    try {
      diskBytes = (await stat(dbPath)).size;
    } catch {
      // file might have been cleaned
    }

    return calculateStats(latenciesUs, workload, totalDurationMs, {
      surface: "Node.js (Node-API) Embedded",
      codec,
      workload,
      batchSize,
      diskBytes,
    });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}
