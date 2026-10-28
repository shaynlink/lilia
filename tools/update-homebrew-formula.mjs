#!/usr/bin/env node

/**
 * Update Homebrew Formula with release archive checksums.
 *
 * Usage:
 *   node tools/update-homebrew-formula.mjs [--version <version>] [--dir <release-dir>] [--formula <path>]
 */

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";

function parseArgs() {
  const args = process.argv.slice(2);
  let version = null;
  let releaseDir = "release";
  let formulaPath = "Formula/lilia.rb";

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--version" && i + 1 < args.length) {
      version = args[++i].replace(/^v/, "");
    } else if (args[i] === "--dir" && i + 1 < args.length) {
      releaseDir = args[++i];
    } else if (args[i] === "--formula" && i + 1 < args.length) {
      formulaPath = args[++i];
    }
  }

  return { version, releaseDir, formulaPath };
}

function computeSha256(filePath) {
  const fileBuffer = readFileSync(filePath);
  return createHash("sha256").update(fileBuffer).digest("hex");
}

function main() {
  const rootDir = fileURLToPath(new URL("..", import.meta.url));
  const { version, releaseDir, formulaPath } = parseArgs();

  const fullFormulaPath = resolve(rootDir, formulaPath);
  if (!existsSync(fullFormulaPath)) {
    console.error(`Error: Formula file not found at ${fullFormulaPath}`);
    process.exit(1);
  }

  let formulaContent = readFileSync(fullFormulaPath, "utf8");

  if (version) {
    console.log(`Updating Formula version to ${version}...`);
    formulaContent = formulaContent.replace(
      /version\s+"[^"]+"/,
      `version "${version}"`
    );
  }

  const targets = [
    {
      name: "darwin-arm64",
      file: "lilia-darwin-arm64.tar.gz",
      regex: /(url\s+"[^"]+lilia-darwin-arm64\.tar\.gz"\s*\n\s*sha256\s+")[a-f0-9]+(")/,
    },
    {
      name: "darwin-x64",
      file: "lilia-darwin-x64.tar.gz",
      regex: /(url\s+"[^"]+lilia-darwin-x64\.tar\.gz"\s*\n\s*sha256\s+")[a-f0-9]+(")/,
    },
    {
      name: "linux-arm64-gnu",
      file: "lilia-linux-arm64-gnu.tar.gz",
      regex: /(url\s+"[^"]+lilia-linux-arm64-gnu\.tar\.gz"\s*\n\s*sha256\s+")[a-f0-9]+(")/,
    },
    {
      name: "linux-x64-gnu",
      file: "lilia-linux-x64-gnu.tar.gz",
      regex: /(url\s+"[^"]+lilia-linux-x64-gnu\.tar\.gz"\s*\n\s*sha256\s+")[a-f0-9]+(")/,
    },
  ];

  const fullReleaseDir = resolve(rootDir, releaseDir);
  console.log(`Scanning release directory: ${fullReleaseDir}`);

  let updatedCount = 0;
  for (const target of targets) {
    const archivePath = join(fullReleaseDir, target.file);
    if (existsSync(archivePath)) {
      const sha256 = computeSha256(archivePath);
      console.log(`  ✓ ${target.name} (${target.file}): ${sha256}`);
      if (target.regex.test(formulaContent)) {
        formulaContent = formulaContent.replace(target.regex, `$1${sha256}$2`);
        updatedCount++;
      } else {
        console.warn(`    Warning: Pattern for ${target.name} not found in formula.`);
      }
    } else {
      console.log(`  - ${target.name} (${target.file}) not found in ${releaseDir}`);
    }
  }

  writeFileSync(fullFormulaPath, formulaContent, "utf8");
  console.log(`Successfully updated ${updatedCount} target(s) in ${formulaPath}`);
}

main();
