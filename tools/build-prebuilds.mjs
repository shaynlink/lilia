#!/usr/bin/env node

/**
 * Multi-Platform Prebuild Builder for LiliaDB Node SDK (@liliadb/node)
 *
 * Usage:
 *   node tools/build-prebuilds.mjs [--target <target>] [--pack] [--dest <dir>]
 *
 * Supported Targets:
 *   - aarch64-apple-darwin      -> @liliadb/node-darwin-arm64
 *   - x86_64-apple-darwin       -> @liliadb/node-darwin-x64
 *   - x86_64-unknown-linux-gnu  -> @liliadb/node-linux-x64-gnu
 *   - aarch64-unknown-linux-gnu -> @liliadb/node-linux-arm64-gnu
 *   - x86_64-pc-windows-msvc    -> @liliadb/node-win32-x64-msvc
 */

import { execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { resolve, join } from "node:path";
import { createRequire } from "node:module";

const TARGET_MAP = {
  "aarch64-apple-darwin": {
    package: "node-darwin-arm64",
    libName: "liblilia_node_native.dylib",
  },
  "x86_64-apple-darwin": {
    package: "node-darwin-x64",
    libName: "liblilia_node_native.dylib",
  },
  "x86_64-unknown-linux-gnu": {
    package: "node-linux-x64-gnu",
    libName: "liblilia_node_native.so",
  },
  "aarch64-unknown-linux-gnu": {
    package: "node-linux-arm64-gnu",
    libName: "liblilia_node_native.so",
  },
  "x86_64-pc-windows-msvc": {
    package: "node-win32-x64-msvc",
    libName: "lilia_node_native.dll",
  },
};

function detectHostTarget() {
  const { platform, arch } = process;
  if (platform === "darwin") {
    return arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
  }
  if (platform === "linux") {
    return arch === "arm64" ? "aarch64-unknown-linux-gnu" : "x86_64-unknown-linux-gnu";
  }
  if (platform === "win32") {
    return "x86_64-pc-windows-msvc";
  }
  throw new Error(`Unsupported host platform: ${platform} (${arch})`);
}

function parseArgs() {
  const args = process.argv.slice(2);
  let target = null;
  let pack = false;
  let dest = null;
  let skipBuild = false;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--target" && i + 1 < args.length) {
      target = args[++i];
    } else if (args[i] === "--pack") {
      pack = true;
    } else if (args[i] === "--dest" && i + 1 < args.length) {
      dest = args[++i];
    } else if (args[i] === "--skip-build") {
      skipBuild = true;
    }
  }

  return { target: target || detectHostTarget(), pack, dest, skipBuild };
}

function main() {
  const rootDir = resolve(new URL("..", import.meta.url).pathname);
  const { target, pack, dest, skipBuild } = parseArgs();

  const config = TARGET_MAP[target];
  if (!config) {
    console.error(`Unknown target triple: ${target}`);
    console.error(`Available targets: ${Object.keys(TARGET_MAP).join(", ")}`);
    process.exit(1);
  }

  const hostTarget = detectHostTarget();
  const isCross = target !== hostTarget;

  // Locate output binary first
  const candidatePaths = [
    join(rootDir, "target", target, "release", config.libName),
    join(rootDir, "target", "release", config.libName),
  ];

  let sourceLib = null;
  for (const candidate of candidatePaths) {
    if (existsSync(candidate)) {
      sourceLib = candidate;
      break;
    }
  }

  if (!sourceLib && !skipBuild) {
    console.log(`=== Building LiliaDB prebuild for ${target} ===`);

    // Build release binary
    const buildCmd = isCross
      ? `cargo build -p lilia-node-native --release --target ${target}`
      : `cargo build -p lilia-node-native --release`;

    console.log(`> ${buildCmd}`);
    execSync(buildCmd, { cwd: rootDir, stdio: "inherit", shell: true });

    for (const candidate of candidatePaths) {
      if (existsSync(candidate)) {
        sourceLib = candidate;
        break;
      }
    }
  }

  if (!sourceLib) {
    console.error(`Error: Could not locate built library. Looked at:`);
    candidatePaths.forEach((p) => console.error(`  - ${p}`));
    process.exit(1);
  }

  const stat = statSync(sourceLib);
  console.log(`Found built binary: ${sourceLib} (${(stat.size / 1024 / 1024).toFixed(2)} MB)`);

  // Target package staging
  const targetPkgDir = join(rootDir, "packages", config.package);
  mkdirSync(targetPkgDir, { recursive: true });
  const stagedPath = join(targetPkgDir, "lilia_node_native.node");

  console.log(`Staging prebuild to: ${stagedPath}`);
  copyFileSync(sourceLib, stagedPath);

  // Smoke test if native host
  if (!isCross) {
    console.log(`Testing native prebuild loading...`);
    try {
      const require = createRequire(import.meta.url);
      const mod = require(stagedPath);
      if (mod && mod.NativeDatabase) {
        console.log(`Native module loaded successfully.`);
      } else {
        throw new Error("Module did not export NativeDatabase");
      }
    } catch (err) {
      console.error(`Smoke test failed:`, err);
      process.exit(1);
    }
  }

  // Pack package if requested
  if (pack) {
    const packDest = dest ? resolve(rootDir, dest) : join(rootDir, "release");
    mkdirSync(packDest, { recursive: true });
    console.log(`Packing @liliadb/${config.package} into ${packDest}...`);
    execSync(`pnpm --dir "${targetPkgDir}" pack --pack-destination "${packDest}"`, {
      cwd: rootDir,
      stdio: "inherit",
      shell: true,
    });
  }

  console.log(`Prebuild for ${target} completed successfully.`);
}

main();
