import assert from "node:assert/strict";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect } from "node:net";
import { spawn, type ChildProcess } from "node:child_process";
import test from "node:test";

import { Database, LiliaError, type DatabaseMode } from "./index.js";

const nativePath = process.env.LILIA_NATIVE_PATH;
const daemonBinary = process.env.LILIA_DAEMON_BIN;

test("close timeout drains accepted JS calls and can be awaited again", async () => {
  let release!: (value: []) => void;
  let closes = 0;
  const delayed = new Promise<[]>((resolve) => { release = resolve; });
  // Inject a deterministic delayed client through the runtime constructor. No
  // native timing assumptions are needed to exercise the SDK admission boundary.
  const database = Reflect.construct(Database, [{
    batch: () => delayed,
    close: async () => { closes += 1; },
  }]) as Database;
  await assert.rejects(database.close({ timeoutMs: -1 }), RangeError);
  const accepted = database.batch([]);
  await assert.rejects(database.close({ timeoutMs: 0 }), (error: unknown) =>
    error instanceof LiliaError && error.code === "TIMEOUT" && error.retryable && !!error.requestId);
  assert.equal(closes, 0);
  await assert.rejects(database.batch([]), (error: unknown) =>
    error instanceof LiliaError && error.code === "CLOSED" && !!error.requestId);
  release([]);
  await accepted;
  await Promise.all([database.close(), database.close()]);
  assert.equal(closes, 1);
});

test("embedded close drains submitted writes and releases storage", { skip: !nativePath }, async () => {
  const directory = await mkdtemp(join(tmpdir(), "lilia-close-"));
  const path = join(directory, "db.lilia");
  try {
    const database = await Database.open({ path });
    const writes = Array.from({ length: 16 }, (_, index) =>
      database.json.put("drain", String(index), { index }));
    await database.close();
    await Promise.all(writes);
    const reopened = await Database.open({ path });
    try {
      assert.equal((await reopened.json.scan("drain")).length, 16);
      assert.equal(await reopened.integrityCheck(), true);
    } finally { await reopened.close(); }
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("checkpoint policy rejects invalid values before opening native storage", async () => {
  for (const value of [0, -1, 0.5, Number.NaN, Number.POSITIVE_INFINITY, 2 ** 32]) {
    await assert.rejects(Database.open({ path: "unused.lilia", checkpointPolicy: { walBytes: value } }), RangeError);
    await assert.rejects(Database.open({ path: "unused.lilia", checkpointPolicy: { intervalMs: value } }), RangeError);
  }
});

test("embedded KV, JSON, and rollback conformance", { skip: !nativePath }, async () => {
  const directory = await mkdtemp(join(tmpdir(), "lilia-node-"));
  try { await conformance("embedded", join(directory, "embedded.lilia")); }
  finally { await rm(directory, { recursive: true, force: true }); }
});

test("daemon KV and JSON conformance", { skip: !daemonBinary }, async () => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "lilia-daemon-")));
  const path = join(directory, "daemon.lilia");
  const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\liliadb-test-${process.pid}` : `${path}.sock`;
  const tokenFile = process.platform === "win32" ? join(directory, "ipc", "token") : `${path}.token`;
  let daemon: ChildProcess | undefined;
  try {
    daemon = spawn(daemonBinary!, ["--database", path, "--endpoint", endpoint, "--token-file", tokenFile], {
      stdio: "ignore",
    });
    await waitUntilReady(tokenFile, endpoint, daemon);
    await conformance("daemon", path, endpoint, tokenFile);
  } finally {
    daemon?.kill();
    await rm(directory, { recursive: true, force: true });
  }
});

async function conformance(mode: DatabaseMode, path: string, endpoint?: string, tokenFile?: string): Promise<void> {
  const database = await Database.open({ path, mode, endpoint, tokenFile,
    checkpointPolicy: mode === "embedded" ? { walBytes: 4096, intervalMs: 25 } : undefined });
  const bytes = new TextEncoder();
  try {
    const write = await database.kv.set("cache", bytes.encode("answer"), bytes.encode("42"), { ifVersion: 0n });
    assert.equal(write.version, 1n);
    assert.equal(new TextDecoder().decode((await database.kv.get("cache", bytes.encode("answer")))?.value), "42");
    const document = await database.json.put("people", "ada", { name: "Ada" }, { ifVersion: 0n });
    assert.equal(document.version, 1n);
    assert.deepEqual((await database.json.get("people", "ada"))?.value, { name: "Ada" });
    await assert.rejects(
      database.batch([
        { model: "json_put", space: "people", id: "grace", value: { name: "Grace" } },
        { model: "json_delete", space: "people", id: "missing", ifVersion: 1n },
      ]),
      (error: unknown) => error instanceof LiliaError || error instanceof Error,
    );
    assert.equal(await database.json.get("people", "grace"), undefined);
    assert.equal(await database.integrityCheck(), true);
  } finally {
    await database.close();
  }
  await assert.rejects(database.integrityCheck(), (error: unknown) =>
    error instanceof LiliaError && error.code === "CLOSED");
}

async function waitUntilReady(tokenFile: string, endpoint: string, daemon: ChildProcess): Promise<void> {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    if (daemon.exitCode !== null) throw new Error(`daemon exited with ${daemon.exitCode}`);
    try {
      const { access } = await import("node:fs/promises");
      await access(tokenFile);
      // The token is created before SQLite is opened and the IPC listener is bound.
      await new Promise<void>((resolve, reject) => {
        const socket = connect(endpoint);
        socket.once("connect", () => { socket.destroy(); resolve(); });
        socket.once("error", error => { socket.destroy(); reject(error); });
      });
      return;
    } catch {
      await new Promise(resolve => setTimeout(resolve, 25));
    }
  }
  throw new Error("daemon startup timed out");
}
