#!/usr/bin/env node

/**
 * LiliaDB Node.js SDK — Local Daemon Mode Example
 *
 * Demonstrates:
 * - Connecting to a running LiliaDB daemon via Unix domain socket / Windows named pipe
 * - Performing Key/Value and JSON operations over IPC
 * - Streaming over IPC
 */

import { Database } from "../packages/node/dist/index.js";
import { spawn } from "node:child_process";
import { access, mkdtemp, realpath, rm } from "node:fs/promises";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

async function main() {
  const tempDir = await realpath(await mkdtemp(join(tmpdir(), "liliadb-daemon-demo-")));
  const dbPath = join(tempDir, "daemon.lilia");
  const isWindows = process.platform === "win32";

  const endpoint = isWindows
    ? `\\\\.\\pipe\\liliadb-example-${process.pid}`
    : `${dbPath}.sock`;
  const tokenFile = isWindows
    ? join(tempDir, "ipc", "token")
    : `${dbPath}.token`;

  const daemonBinary = process.env.LILIA_DAEMON_BIN ?? new URL("../target/debug/lilia-daemon", import.meta.url).pathname;

  console.log(`[1] Launching local daemon process...`);
  console.log(`    Binary:   ${daemonBinary}`);
  console.log(`    Endpoint: ${endpoint}`);

  const daemon = spawn(daemonBinary, [
    "--database", dbPath,
    "--endpoint", endpoint,
    "--token-file", tokenFile,
  ], { stdio: "ignore" });

  try {
    // Wait for the daemon to create the security token and bind the IPC socket
    await waitUntilReady(tokenFile, endpoint, daemon);
    console.log(`[2] Daemon is ready and listening.`);

    // ----------------------------------------------------
    // Connect in daemon mode
    // ----------------------------------------------------
    console.log(`\n[3] Connecting via Database.open(mode: "daemon")...`);
    const db = await Database.open({
      mode: "daemon",
      path: dbPath,
      endpoint,
      tokenFile,
    });

    try {
      // Key/Value over IPC
      console.log("\n[4] Performing Key/Value operations over daemon IPC:");
      const encoder = new TextEncoder();
      const decoder = new TextDecoder();

      await db.kv.set("settings", encoder.encode("theme"), encoder.encode("dark"));
      const theme = await db.kv.get("settings", encoder.encode("theme"));
      console.log(`    Theme setting: ${decoder.decode(theme?.value)} (v${theme?.version})`);

      // JSON Document over IPC
      console.log("\n[5] Performing JSON Document operations over daemon IPC:");
      await db.json.put("servers", "web-01", { host: "10.0.0.1", region: "eu-west-1", online: true });
      const srv = await db.json.get("servers", "web-01");
      console.log(`    Server web-01:`, srv?.value);

      // Streaming over IPC
      console.log("\n[6] Streaming documents over daemon IPC:");
      for (let i = 1; i <= 5; i++) {
        await db.json.put("servers", `worker-0${i}`, { host: `10.0.0.1${i}`, active: true });
      }

      for await (const server of db.json.scanStream("servers", { batchSize: 3 })) {
        console.log(`    - Server: ${server.id} (host: ${server.value?.host})`);
      }

    } finally {
      console.log("\n[7] Closing client session...");
      await db.close();
    }
  } finally {
    console.log("\n[8] Stopping daemon and cleaning up...");
    daemon.kill();
    await rm(tempDir, { recursive: true, force: true });
    console.log("Done!");
  }
}

async function waitUntilReady(tokenFile, endpoint, daemon) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (daemon.exitCode !== null) {
      throw new Error(`daemon exited early with code ${daemon.exitCode}`);
    }
    try {
      await access(tokenFile);
      await new Promise((resolve, reject) => {
        const socket = connect(endpoint);
        socket.once("connect", () => { socket.destroy(); resolve(); });
        socket.once("error", (err) => { socket.destroy(); reject(err); });
      });
      return;
    } catch {
      await new Promise(r => setTimeout(r, 25));
    }
  }
  throw new Error("daemon startup timed out");
}

main().catch((err) => {
  console.error("Error in daemon example:", err);
  process.exit(1);
});
