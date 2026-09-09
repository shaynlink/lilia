import { createRequire } from "node:module";
import type { BatchOperation, JsonEntry, KvEntry, MutationResult } from "./types.js";

interface NativeKvEntry { namespace: string; key: Buffer; value: Buffer; version: string; expiresAtMs?: number }
interface NativeDatabaseHandle {
  kvGet(namespace: string, key: Buffer): Promise<NativeKvEntry | null>;
  kvScan(namespace: string, after: Buffer | undefined, limit: number): Promise<NativeKvEntry[]>;
  jsonGet(space: string, id: string): Promise<string | null>;
  jsonScan(space: string, after: string | undefined, limit: number): Promise<string>;
  batch(operations: string): Promise<string>;
  integrityCheck(): Promise<boolean>;
}
interface NativeModule { NativeDatabase: { open(path: string): Promise<NativeDatabaseHandle> } }

const require = createRequire(import.meta.url);

export class EmbeddedClient {
  private constructor(private readonly native: NativeDatabaseHandle) {}

  static async open(path: string): Promise<EmbeddedClient> {
    const module = loadNative();
    return new EmbeddedClient(await module.NativeDatabase.open(path));
  }

  async kvGet(namespace: string, key: Uint8Array): Promise<KvEntry | undefined> {
    const entry = await this.native.kvGet(namespace, Buffer.from(key));
    return entry ? mapKv(entry) : undefined;
  }

  async kvScan(namespace: string, after: Uint8Array | undefined, limit: number): Promise<KvEntry[]> {
    return (await this.native.kvScan(namespace, after ? Buffer.from(after) : undefined, limit)).map(mapKv);
  }

  async jsonGet(space: string, id: string): Promise<JsonEntry | undefined> {
    const entry = await this.native.jsonGet(space, id);
    return entry ? parseJsonEntry(entry) : undefined;
  }

  async jsonScan(space: string, after: string | undefined, limit: number): Promise<JsonEntry[]> {
    return (JSON.parse(await this.native.jsonScan(space, after, limit)) as Array<Record<string, unknown>>).map(mapJson);
  }

  async batch(operations: readonly BatchOperation[]): Promise<MutationResult[]> {
    const wire = operations.map(toNativeWire);
    const result = JSON.parse(await this.native.batch(JSON.stringify(wire))) as Array<{ version?: number; deleted: boolean }>;
    return result.map(item => ({ ...item, version: item.version === undefined ? undefined : BigInt(item.version) }));
  }

  integrityCheck(): Promise<boolean> { return this.native.integrityCheck(); }
  close(): Promise<void> { return Promise.resolve(); }
}

function loadNative(): NativeModule {
  const explicit = process.env.LILIA_NATIVE_PATH;
  if (explicit) return require(explicit) as NativeModule;
  const platform = `${process.platform}-${process.arch}`;
  try { return require(`@liliadb/node-${platform}`) as NativeModule; }
  catch (error) { throw new Error(`No LiliaDB native build for ${platform}; set LILIA_NATIVE_PATH`, { cause: error }); }
}

function mapKv(entry: NativeKvEntry): KvEntry {
  return { namespace: entry.namespace, key: entry.key, value: entry.value, version: BigInt(entry.version), expiresAtMs: entry.expiresAtMs };
}
function parseJsonEntry(value: string): JsonEntry { return mapJson(JSON.parse(value) as Record<string, unknown>); }
function mapJson(entry: Record<string, unknown>): JsonEntry {
  return { space: String(entry.space), id: String(entry.id), value: entry.value as JsonEntry["value"], version: BigInt(entry.version as number) };
}
function toNativeWire(operation: BatchOperation): Record<string, unknown> {
  const result = { ...operation } as Record<string, unknown>;
  if ("ifVersion" in operation) {
    result.if_version = operation.ifVersion === undefined ? undefined : Number(operation.ifVersion);
    delete result.ifVersion;
  }
  if ("expiresAtMs" in operation) {
    result.expires_at_ms = operation.expiresAtMs;
    delete result.expiresAtMs;
  }
  if (operation.model === "kv_set" || operation.model === "kv_delete") {
    result.key = [...operation.key];
    if (operation.model === "kv_set") result.value = [...operation.value];
  }
  return result;
}
