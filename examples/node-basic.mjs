#!/usr/bin/env node

/**
 * LiliaDB Node.js SDK — Basic Usage Example
 *
 * Demonstrates:
 * - Opening an embedded database
 * - Key/Value storage with text and binary keys/values
 * - JSON Document storage
 * - Atomic multi-model batch transactions with optimistic concurrency (ifVersion)
 * - Safe lifecycle closing
 */

import { Database } from "../packages/node/dist/index.js";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

async function main() {
  const tempDir = await mkdtemp(join(tmpdir(), "liliadb-basic-"));
  const dbPath = join(tempDir, "app.lilia");

  console.log(`[1] Opening embedded LiliaDB at ${dbPath}...`);
  const db = await Database.open({
    path: dbPath,
    durability: "durable",
  });

  try {
    // ----------------------------------------------------
    // Key/Value Model
    // ----------------------------------------------------
    console.log("\n[2] Key/Value Operations:");
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();

    const apiKeyKey = encoder.encode("services:stripe:api_key");
    const apiKeyValue = encoder.encode("sk_live_51MzExampleSecretToken");

    // Write a key
    const writeResult = await db.kv.set("credentials", apiKeyKey, apiKeyValue);
    console.log(`  Set 'services:stripe:api_key' (version: ${writeResult.version})`);

    // Read the key back
    const entry = await db.kv.get("credentials", apiKeyKey);
    if (entry) {
      console.log(`  Get 'services:stripe:api_key': ${decoder.decode(entry.value)} (v${entry.version})`);
    }

    // ----------------------------------------------------
    // JSON Document Model
    // ----------------------------------------------------
    console.log("\n[3] JSON Document Operations:");
    const userDoc = {
      name: "Ada Lovelace",
      role: "pioneer",
      skills: ["mathematics", "computing", "algorithm design"],
      created_at: Date.now(),
    };

    // Put document
    const putResult = await db.json.put("users", "ada", userDoc);
    console.log(`  Put user 'ada' (version: ${putResult.version})`);

    // Get document
    const user = await db.json.get("users", "ada");
    console.log(`  Get user 'ada':`, user?.value);

    // ----------------------------------------------------
    // Multi-Model Atomic Batch (KV + JSON in 1 commit)
    // ----------------------------------------------------
    console.log("\n[4] Multi-Model Atomic Batch Transaction:");
    const batchResults = await db.batch([
      {
        model: "json_put",
        space: "users",
        id: "ada",
        value: { ...userDoc, last_login: Date.now() },
        ifVersion: putResult.version, // Optimistic concurrency check
      },
      {
        model: "kv_set",
        namespace: "sessions",
        key: encoder.encode("session_001"),
        value: encoder.encode("active"),
        expiresAtMs: Date.now() + 3600_000, // 1 hour TTL
      },
    ]);
    console.log(`  Batch committed ${batchResults.length} operations atomically:`);
    batchResults.forEach((res, i) => console.log(`    Operation ${i + 1}: version ${res.version}`));

    // ----------------------------------------------------
    // Database Health Check
    // ----------------------------------------------------
    console.log("\n[5] Integrity Check:");
    const isHealthy = await db.integrityCheck();
    console.log(`  Storage integrity status: ${isHealthy ? "HEALTHY" : "CORRUPT"}`);
  } finally {
    // ----------------------------------------------------
    // Safe Closing
    // ----------------------------------------------------
    console.log("\n[6] Closing database cleanly...");
    await db.close({ timeoutMs: 5_000 });
    await rm(tempDir, { recursive: true, force: true });
    console.log("Done! Sample database cleaned up.");
  }
}

main().catch((err) => {
  console.error("Error running basic example:", err);
  process.exit(1);
});
