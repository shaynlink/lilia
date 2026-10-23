#!/usr/bin/env node

/**
 * LiliaDB Node.js SDK — Streaming & Large Scans Example
 *
 * Demonstrates:
 * - Ingesting a large dataset in bulk batches
 * - Asynchronous iteration (for await) with automatic keyset pagination
 * - Controlling batchSize and global limit
 * - Converting scans to standard Node.js Readable streams (createReadStream)
 * - Piping through stream pipelines and transforms
 */

import { Database } from "../packages/node/dist/index.js";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Transform } from "node:stream";
import { pipeline } from "node:stream/promises";

async function main() {
  const tempDir = await mkdtemp(join(tmpdir(), "liliadb-streaming-"));
  const dbPath = join(tempDir, "stream.lilia");

  console.log(`[1] Opening LiliaDB for streaming demo at ${dbPath}...`);
  const db = await Database.open({ path: dbPath });

  try {
    // ----------------------------------------------------
    // Seed sample data: 300 telemetry events
    // ----------------------------------------------------
    const TOTAL_RECORDS = 300;
    console.log(`\n[2] Seeding ${TOTAL_RECORDS} JSON telemetry events...`);

    const encoder = new TextEncoder();
    const batchOps = [];

    for (let i = 0; i < TOTAL_RECORDS; i++) {
      const id = `event_${String(i).padStart(4, "0")}`;
      batchOps.push({
        model: "json_put",
        space: "telemetry",
        id,
        value: {
          sensor_id: `sensor_${i % 10}`,
          temperature_c: 20 + (i % 15) + Math.random(),
          timestamp: Date.now() + i * 1000,
        },
      });
      // Also write corresponding KV index
      batchOps.push({
        model: "kv_set",
        namespace: "indexes",
        key: encoder.encode(`idx:${id}`),
        value: encoder.encode(id),
      });
    }

    // Insert in batches of 100 operations
    for (let i = 0; i < batchOps.length; i += 100) {
      await db.batch(batchOps.slice(i, i + 100));
    }
    console.log(`  Seeded ${TOTAL_RECORDS} records successfully.`);

    // ----------------------------------------------------
    // 1. Asynchronous Iteration (for await) on JSON Documents
    // ----------------------------------------------------
    console.log("\n[3] Streaming JSON documents via for-await with batchSize = 50:");
    let jsonCount = 0;
    let firstId = "";
    let lastId = "";

    // Automatically pages every 50 records behind the scenes without manual while-loops
    for await (const doc of db.json.scanStream("telemetry", { batchSize: 50 })) {
      if (jsonCount === 0) firstId = doc.id;
      lastId = doc.id;
      jsonCount++;
    }
    console.log(`  Streamed all ${jsonCount} documents from '${firstId}' to '${lastId}'`);

    // ----------------------------------------------------
    // 2. Controlled scan with global limit
    // ----------------------------------------------------
    console.log("\n[4] Streaming with global limit = 75 (batchSize = 20):");
    const sampleIds = [];
    for await (const doc of db.json.scanStream("telemetry", { batchSize: 20, limit: 75 })) {
      sampleIds.push(doc.id);
    }
    console.log(`  Yielded exactly ${sampleIds.length} items (First: ${sampleIds[0]}, Last: ${sampleIds[sampleIds.length - 1]})`);

    // ----------------------------------------------------
    // 3. Streaming Key/Value pairs with resume cursor (after)
    // ----------------------------------------------------
    console.log("\n[5] Streaming KV index keys resuming strictly after 'idx:event_0100':");
    const resumeCursor = encoder.encode("idx:event_0100");
    let kvCount = 0;
    let firstKey = "";

    for await (const entry of db.kv.scanStream("indexes", { after: resumeCursor, batchSize: 50, limit: 5 })) {
      const keyStr = new TextDecoder().decode(entry.key);
      if (kvCount === 0) firstKey = keyStr;
      console.log(`    ${keyStr} -> ${new TextDecoder().decode(entry.value)}`);
      kvCount++;
    }
    console.log(`  Streamed ${kvCount} entries starting after event_0100`);

    // ----------------------------------------------------
    // 4. Node.js Streams Pipeline (createReadStream)
    // ----------------------------------------------------
    console.log("\n[6] Using Node.js Streams Pipeline with createReadStream():");
    let totalTemp = 0;
    let validCount = 0;

    const readable = db.json.createReadStream("telemetry", { batchSize: 50 });

    const aggregator = new Transform({
      objectMode: true,
      transform(doc, _encoding, callback) {
        if (doc?.value?.temperature_c) {
          totalTemp += doc.value.temperature_c;
          validCount++;
        }
        callback();
      },
    });

    await pipeline(readable, aggregator);
    const avgTemp = (totalTemp / validCount).toFixed(2);
    console.log(`  Aggregated ${validCount} documents via stream pipeline. Average temperature: ${avgTemp}°C`);

  } finally {
    console.log("\n[7] Cleaning up and closing database...");
    await db.close();
    await rm(tempDir, { recursive: true, force: true });
    console.log("Done!");
  }
}

main().catch((err) => {
  console.error("Error in streaming example:", err);
  process.exit(1);
});
