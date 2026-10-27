import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, realpath, rm, stat } from "node:fs/promises";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { Database, type BatchOperation } from "../../index.js";
import { calculateStats, type BenchmarkResult } from "../stats.js";

async function waitUntilReady(
  tokenFile: string,
  endpoint: string,
  child: ChildProcess
): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (child.exitCode != null) {
      throw new Error(`Daemon exited prematurely with code ${child.exitCode}`);
    }
    if (existsSync(tokenFile)) {
      try {
        await new Promise<void>((resolvePromise, reject) => {
          const socket = connect(endpoint);
          socket.once("connect", () => {
            socket.destroy();
            resolvePromise();
          });
          socket.once("error", reject);
        });
        return;
      } catch {
        // Retry until connectable
      }
    }
    await new Promise((r) => setTimeout(r, 20));
  }
  throw new Error("Timed out waiting for daemon IPC readiness");
}

export async function runDaemonIpcBenchmark(
  workload: number,
  batchSize: number,
  codec: "cleartext" | "aes-256-gcm" = "cleartext"
): Promise<BenchmarkResult> {
  const binaryCandidates = [
    resolve(process.cwd(), "../../target/release/lilia-daemon"),
    resolve(process.cwd(), "../../target/release/lilia-daemon.exe"),
    resolve(process.cwd(), "target/release/lilia-daemon"),
    resolve(process.cwd(), "target/release/lilia-daemon.exe"),
  ];
  const binary = binaryCandidates.find(existsSync);
  if (!binary) {
    throw new Error(
      `Daemon binary 'lilia-daemon' not found. Run 'cargo build --release -p lilia-daemon' first.`
    );
  }

  const dir = await realpath(await mkdtemp(join(tmpdir(), "l-ib-")));
  const dbPath = join(dir, "d.lilia");
  const endpoint =
    process.platform === "win32"
      ? `\\\\.\\pipe\\lilia-bench-${process.pid}-${Date.now()}`
      : join(dir, "d.sock");
  const tokenFile =
    process.platform === "win32"
      ? join(dir, "ipc", "token")
      : join(dir, "d.token");

  const daemonArgs = [
    "--database",
    dbPath,
    "--endpoint",
    endpoint,
    "--token-file",
    tokenFile,
  ];
  if (codec === "aes-256-gcm") {
    daemonArgs.push("--encryption-key", "liliadb-benchmark-official-key");
  }

  const daemon = spawn(binary, daemonArgs, { stdio: "ignore" });

  try {
    await waitUntilReady(tokenFile, endpoint, daemon);

    const db = await Database.open({
      path: dbPath,
      mode: "daemon",
      endpoint,
      tokenFile,
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

      daemon.kill();
      await new Promise((r) => setTimeout(r, 50));

      let diskBytes: number | undefined;
      try {
        const main = (await stat(dbPath)).size;
        let wal = 0;
        try {
          wal = (await stat(`${dbPath}-wal`)).size;
        } catch {}
        diskBytes = main + wal;
      } catch {
        // file might be deleted
      }

      return calculateStats(latenciesUs, workload, totalDurationMs, {
        surface: "Daemon via IPC (Socket / Pipe)",
        codec,
        workload,
        batchSize,
        diskBytes,
      });
    } finally {
      await db.close().catch(() => {});
    }
  } finally {
    daemon.kill();
    await rm(dir, { recursive: true, force: true }).catch(() => {});
  }
}
