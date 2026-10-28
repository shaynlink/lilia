import assert from "node:assert/strict";
import { mkdtemp, realpath, rm, access } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect } from "node:net";
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";

import { Database } from "../src/index.ts";

// Universal test declaration for Deno test runner and Node test runner
const runTest = typeof (globalThis as any).Deno !== "undefined"
  ? (name: string, fn: () => Promise<void>) => (globalThis as any).Deno.test(name, fn)
  : (await import("node:test")).default;

const candidateDaemon = [
  new URL("../../../target/debug/lilia-daemon", import.meta.url).pathname,
  new URL("../../../target/release/lilia-daemon", import.meta.url).pathname,
].find((p) => existsSync(p));

const daemonBinary = process.env.LILIA_DAEMON_BIN ?? candidateDaemon;

async function waitUntilReady(tokenFile: string, endpoint: string, daemon: ChildProcess): Promise<void> {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    if (daemon.exitCode !== null) throw new Error(`daemon exited with ${daemon.exitCode}`);
    try {
      await access(tokenFile);
      await new Promise<void>((resolve, reject) => {
        const socket = connect(endpoint);
        socket.once("connect", () => { socket.destroy(); resolve(); });
        socket.once("error", (error) => { socket.destroy(); reject(error); });
      });
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
  }
  throw new Error("daemon startup timed out");
}

runTest("Deno: daemon client connects over Unix domain socket IPC", async () => {
  if (!daemonBinary) {
    console.warn("Skipping Deno daemon test: lilia-daemon binary not found");
    return;
  }

  const dir = await realpath(await mkdtemp(join(tmpdir(), "deno-dae-")));
  const dbPath = join(dir, "deno.lilia");
  const endpoint = `${dbPath}.sock`;
  const tokenFile = `${dbPath}.token`;

  const daemonProc = spawn(
    daemonBinary,
    ["--database", dbPath, "--endpoint", endpoint, "--token-file", tokenFile],
    { stdio: "ignore" }
  );

  try {
    await waitUntilReady(tokenFile, endpoint, daemonProc);

    const db = await Database.open({
      path: dbPath,
      mode: "daemon",
      endpoint,
      tokenFile,
    });

    // KV operations under Deno
    const key = new TextEncoder().encode("deno:key1");
    const val = new TextEncoder().encode("deno-value-1");
    const setRes = await db.kv.set("deno_ns", key, val);
    assert.ok(setRes.version !== undefined);

    const got = await db.kv.get("deno_ns", key);
    assert.ok(got !== undefined);
    assert.equal(new TextDecoder().decode(got.value), "deno-value-1");

    // JSON operations under Deno
    const putRes = await db.json.put("deno_docs", "doc-1", { framework: "deno", secure: true });
    assert.ok(putRes.version !== undefined);

    const doc = await db.json.get("deno_docs", "doc-1");
    assert.ok(doc !== undefined);
    assert.equal((doc.value as Record<string, unknown>).framework, "deno");

    await db.close();
  } finally {
    daemonProc.kill("SIGTERM");
    await rm(dir, { recursive: true, force: true });
  }
});
