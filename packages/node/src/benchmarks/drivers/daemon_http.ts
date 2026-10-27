import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, realpath, rm, stat } from "node:fs/promises";
import http from "node:http";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { calculateStats, type BenchmarkResult } from "../stats.js";

async function getAvailablePort(): Promise<number> {
  return new Promise((resolvePromise, reject) => {
    const srv = createServer();
    srv.listen(0, "127.0.0.1", () => {
      const address = srv.address();
      if (address && typeof address === "object") {
        const port = address.port;
        srv.close(() => resolvePromise(port));
      } else {
        srv.close(() => reject(new Error("Failed to get port")));
      }
    });
    srv.on("error", reject);
  });
}

async function waitUntilHttpReady(
  port: number,
  child: ChildProcess
): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (child.exitCode != null) {
      throw new Error(`Daemon exited prematurely with code ${child.exitCode}`);
    }
    try {
      const res = await new Promise<number>((resolvePromise, reject) => {
        const req = http.get(
          `http://127.0.0.1:${port}/health`,
          { timeout: 1000 },
          (response) => {
            response.resume();
            resolvePromise(response.statusCode ?? 0);
          }
        );
        req.on("error", reject);
        req.on("timeout", () => {
          req.destroy();
          reject(new Error("timeout"));
        });
      });
      if (res === 200) {
        return;
      }
    } catch {
      // Retry
    }
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error("Timed out waiting for daemon HTTP gateway readiness");
}

function sendHttpBatch(
  agent: http.Agent,
  port: number,
  token: string,
  operations: unknown[]
): Promise<void> {
  return new Promise((resolvePromise, reject) => {
    const payload = JSON.stringify({ operations });
    const req = http.request(
      `http://127.0.0.1:${port}/v1/batch`,
      {
        method: "POST",
        agent,
        headers: {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(payload),
          Authorization: `Bearer ${token}`,
        },
      },
      (res) => {
        let body = "";
        res.on("data", (chunk: Buffer) => {
          body += chunk.toString("utf8");
        });
        res.on("end", () => {
          if (res.statusCode === 200) {
            resolvePromise();
          } else {
            reject(
              new Error(
                `HTTP batch failed with status ${res.statusCode}: ${body}`
              )
            );
          }
        });
      }
    );
    req.on("error", reject);
    req.write(payload);
    req.end();
  });
}

export async function runDaemonHttpBenchmark(
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

  const dir = await realpath(
    await mkdtemp(join(tmpdir(), "l-hb-"))
  );
  const dbPath = join(dir, "d.lilia");
  const endpoint =
    process.platform === "win32"
      ? `\\\\.\\pipe\\lilia-http-bench-${process.pid}-${Date.now()}`
      : join(dir, "d.sock");
  const tokenFile =
    process.platform === "win32"
      ? join(dir, "ipc", "token")
      : join(dir, "d.token");

  const port = await getAvailablePort();

  const daemonArgs = [
    "--database",
    dbPath,
    "--endpoint",
    endpoint,
    "--token-file",
    tokenFile,
    "--http-port",
    String(port),
  ];
  if (codec === "aes-256-gcm") {
    daemonArgs.push("--encryption-key", "liliadb-benchmark-official-key");
  }

  const daemon = spawn(binary, daemonArgs, { stdio: "ignore" });

  const agent = new http.Agent({
    keepAlive: true,
    maxSockets: 50,
  });

  try {
    await waitUntilHttpReady(port, daemon);
    const token = (await readFile(tokenFile, "utf8")).trim();

    const valueB64 = Buffer.alloc(256, 42).toString("base64");

    // Warmup phase (200 mutations)
    const warmupOps = Array.from({ length: 200 }, (_, i) => ({
      type: "kv_set",
      namespace: "warmup",
      key: Buffer.from(`k_${i}`).toString("base64"),
      value: valueB64,
      encoding: "base64",
    }));
    for (let i = 0; i < warmupOps.length; i += batchSize) {
      await sendHttpBatch(agent, port, token, warmupOps.slice(i, i + batchSize));
    }

    // Timed benchmark phase
    const numBatches = Math.ceil(workload / batchSize);
    const latenciesUs: number[] = new Array<number>(numBatches);

    let generated = 0;
    const startAll = process.hrtime.bigint();

    for (let b = 0; b < numBatches; b++) {
      const currentBatchSize = Math.min(workload - generated, batchSize);
      const ops = new Array(currentBatchSize);
      for (let i = 0; i < currentBatchSize; i++) {
        const id = b * batchSize + i;
        ops[i] = {
          type: "kv_set",
          namespace: "bench",
          key: Buffer.from(`bench_key_${String(id).padStart(8, "0")}`).toString("base64"),
          value: valueB64,
          encoding: "base64",
        };
      }

      const batchStart = process.hrtime.bigint();
      await sendHttpBatch(agent, port, token, ops);
      const batchEnd = process.hrtime.bigint();

      const elapsedNs = Number(batchEnd - batchStart);
      latenciesUs[b] = elapsedNs / (currentBatchSize * 1_000);
      generated += currentBatchSize;
    }

    const endAll = process.hrtime.bigint();
    const totalDurationMs = Number(endAll - startAll) / 1_000_000;

    agent.destroy();
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
      surface: "Daemon via HTTP Loopback",
      codec,
      workload,
      batchSize,
      diskBytes,
    });
  } finally {
    agent.destroy();
    daemon.kill();
    await rm(dir, { recursive: true, force: true }).catch(() => {});
  }
}
