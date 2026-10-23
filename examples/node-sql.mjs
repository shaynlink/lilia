#!/usr/bin/env node

/**
 * LiliaDB Node.js SDK — SQL Declarative Querying Example
 *
 * Demonstrates:
 * - Opening an embedded database
 * - Seeding JSON documents and KV entries
 * - Querying structured data declaratively using SQL via views (lilia_json, lilia_kv)
 * - Using json_extract() to filter and aggregate document fields
 * - Using db.sql.query(), db.sql.raw(), and db.sql.execute()
 * - Safe lifecycle closing and cleanup
 */

import { Database } from "../packages/node/dist/index.js";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

async function main() {
  const tempDir = await mkdtemp(join(tmpdir(), "liliadb-sql-"));
  const dbPath = join(tempDir, "analytics.lilia");

  console.log(`[1] Opening embedded LiliaDB at ${dbPath}...`);
  const db = await Database.open({
    path: dbPath,
    durability: "durable",
  });

  try {
    // ----------------------------------------------------
    // Seed Sample Data
    // ----------------------------------------------------
    console.log("\n[2] Seeding documents and KV data...");
    await db.batch([
      {
        model: "json_put",
        space: "developers",
        id: "ada",
        value: { name: "Ada Lovelace", team: "core", points: 950, active: true },
      },
      {
        model: "json_put",
        space: "developers",
        id: "alan",
        value: { name: "Alan Turing", team: "crypto", points: 880, active: true },
      },
      {
        model: "json_put",
        space: "developers",
        id: "grace",
        value: { name: "Grace Hopper", team: "core", points: 920, active: true },
      },
      {
        model: "json_put",
        space: "developers",
        id: "linus",
        value: { name: "Linus Torvalds", team: "kernel", points: 840, active: false },
      },
    ]);

    const encoder = new TextEncoder();
    await db.kv.set("metrics", encoder.encode("cluster_health"), encoder.encode("green"));
    console.log("  Seeded 4 developers and 1 KV entry.");

    // ----------------------------------------------------
    // 1. Typed SQL Query (db.sql.query)
    // ----------------------------------------------------
    console.log("\n[3] Running SQL query on lilia_json view:");

    const highScorers = await db.sql.query(
      `SELECT
         id,
         json_extract(value, '$.name') AS name,
         json_extract(value, '$.team') AS team,
         json_extract(value, '$.points') AS points
       FROM lilia_json
       WHERE space = ?1 AND json_extract(value, '$.points') >= ?2
       ORDER BY points DESC`,
      ["developers", 900],
    );

    console.log("  High Scorers (points >= 900):");
    for (const dev of highScorers) {
      console.log(`    - ${dev.name} (${dev.team}): ${dev.points} pts [ID: ${dev.id}]`);
    }

    // ----------------------------------------------------
    // 2. Aggregations & Grouping
    // ----------------------------------------------------
    console.log("\n[4] Running SQL aggregations:");

    const teamStats = await db.sql.query(
      `SELECT
         json_extract(value, '$.team') AS team,
         COUNT(*) AS members,
         ROUND(AVG(json_extract(value, '$.points')), 1) AS avgPoints
       FROM lilia_json
       WHERE space = ?1
       GROUP BY team
       ORDER BY avgPoints DESC`,
      ["developers"],
    );

    console.log("  Team Stats:");
    for (const stat of teamStats) {
      console.log(`    - Team '${stat.team}': ${stat.members} members, avg ${stat.avgPoints} pts`);
    }

    // ----------------------------------------------------
    // 3. Raw Projections (db.sql.raw)
    // ----------------------------------------------------
    console.log("\n[5] Running raw projection query:");
    const rawResult = await db.sql.raw(
      `SELECT
         namespace,
         hex(key) AS hex_key,
         CAST(value AS TEXT) AS text_value
       FROM lilia_kv
       WHERE namespace = ?1`,
      ["metrics"],
    );

    console.log(`  Columns: ${rawResult.columns.join(", ")}`);
    console.log(`  Rows (${rawResult.rows.length}):`);
    for (const row of rawResult.rows) {
      console.log(`    ${JSON.stringify(row)}`);
    }

    // ----------------------------------------------------
    // 4. Executing Mutations (db.sql.execute)
    // ----------------------------------------------------
    console.log("\n[6] Mutating data via SQL execute:");
    const mutation = await db.sql.execute(
      `UPDATE _lilia_json
       SET value = json_set(value, '$.points', json_extract(value, '$.points') + 50)
       WHERE space = ?1 AND id = ?2`,
      ["developers", "ada"],
    );
    console.log(`  Rows affected: ${mutation.rowsAffected}`);

    // Verify through the standard JSON API
    const updatedAda = await db.json.get("developers", "ada");
    console.log("  Updated Ada Lovelace points via db.json.get:", updatedAda?.value?.points);

    console.log("\n[✓] SQL example completed successfully!");
  } finally {
    await db.close();
    await rm(tempDir, { recursive: true, force: true });
    console.log(`[7] Closed database and removed ${tempDir}.`);
  }
}

main().catch(err => {
  console.error("Fatal error:", err);
  process.exit(1);
});
