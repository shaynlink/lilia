import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect } from "node:net";
import { spawn, type ChildProcess } from "node:child_process";
import test from "node:test";

import { Database, LiliaError, type DatabaseMode } from "./index.js";

const nativePath = process.env.LILIA_NATIVE_PATH;
const daemonBinary = process.env.LILIA_DAEMON_BIN;

test("embedded KV, JSON, and rollback conformance", { skip: !nativePath }, async () => {
  const directory = await mkdtemp(join(tmpdir(), "lilia-node-"));
  try { await conformance("embedded", join(directory, "embedded.lilia")); }
  finally { await rm(directory, { recursive: true, force: true }); }
});

test("daemon KV and JSON conformance", { skip: !daemonBinary }, async () => {
  const directory = await mkdtemp(join(tmpdir(), "lilia-daemon-"));
  const path = join(directory, "daemon.lilia");
  const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\liliadb-test-${process.pid}` : `${path}.sock`;
  const tokenFile = `${path}.token`;
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
  const database = await Database.open({ path, mode, endpoint, tokenFile });
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
