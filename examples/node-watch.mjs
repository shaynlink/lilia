#!/usr/bin/env node

/**
 * LiliaDB Node.js SDK — Live Event Streaming & CDC Example (db.watch())
 *
 * Demonstrates:
 * - Subscribing to live database change events in real time
 * - Filtering events by namespace across models (KV, JSON, Graph, Fts)
 * - Using db.watch() for reactive cache invalidation & live UI sync
 * - Clean unsubscription and resource teardown
 */

import { Database } from "../packages/node/dist/index.js";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

async function main() {
  const tempDir = await mkdtemp(join(tmpdir(), "lilia-watch-"));
  const dbPath = join(tempDir, "app.lilia");

  console.log(`[1] Opening embedded LiliaDB at ${dbPath}...`);
  const db = await Database.open({
    path: dbPath,
    durability: "durable",
  });

  try {
    // ----------------------------------------------------
    // 1. Reactive In-Memory Cache with db.watch()
    // ----------------------------------------------------
    console.log("\n[2] Setting up real-time reactive cache for 'orders'...");
    const localOrderCache = new Map();

    const decoder = new TextDecoder();
    const orderSub = await db.watch("orders", (event) => {
      const keyOrId = event.id || (event.key ? (typeof event.key === "string" ? event.key : decoder.decode(event.key)) : "");
      console.log(`  [CDC Event] Type: ${event.type} | ID/Key: ${keyOrId} | Deleted: ${event.deleted}`);
      if (event.deleted) {
        localOrderCache.delete(keyOrId);
      } else {
        localOrderCache.set(keyOrId, { version: event.version, lastUpdated: Date.now() });
      }
    });

    // ----------------------------------------------------
    // 2. Perform Mutations in 'orders' (Received)
    // ----------------------------------------------------
    const encoder = new TextEncoder();
    console.log("\n[3] Mutating data in 'orders' namespace...");
    await db.json.put("orders", "ord_101", {
      customer: "Alice",
      total: 149.99,
      status: "pending",
    });

    await db.kv.set("orders", encoder.encode("ord_meta:101"), encoder.encode("priority_rush"));

    // ----------------------------------------------------
    // 3. Perform Mutations in 'inventory' (Filtered Out)
    // ----------------------------------------------------
    console.log("\n[4] Mutating data in 'inventory' (should NOT appear in 'orders' watch)...");
    await db.kv.set("inventory", encoder.encode("sku_laptop_pro"), encoder.encode("stock:42"));

    // Give asynchronous events a moment to process
    await new Promise((resolve) => setTimeout(resolve, 50));

    console.log(`\n[5] Current Local In-Memory Cache entries: ${localOrderCache.size}`);
    for (const [key, val] of localOrderCache.entries()) {
      console.log(`  -> Key: ${key}, Version: ${val.version}`);
    }

    // ----------------------------------------------------
    // 4. Delete Mutation
    // ----------------------------------------------------
    console.log("\n[6] Deleting 'ord_101'...");
    await db.json.delete("orders", "ord_101");
    await new Promise((resolve) => setTimeout(resolve, 50));

    console.log(`[7] Cache entries after deletion: ${localOrderCache.size}`);

    // ----------------------------------------------------
    // 5. Unsubscribe
    // ----------------------------------------------------
    console.log("\n[8] Unsubscribing from 'orders' watch...");
    orderSub.unsubscribe();
    console.log("  Successfully unsubscribed!");

  } finally {
    console.log("\n[9] Closing database and cleaning up temp files...");
    await db.close();
    await rm(tempDir, { recursive: true, force: true });
    console.log("  Done!");
  }
}

main().catch((err) => {
  console.error("Error in node-watch example:", err);
  process.exit(1);
});
