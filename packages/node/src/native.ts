import { createRequire } from "node:module";
import { existsSync, copyFileSync } from "node:fs";
import type {
  BatchOperation,
  DatabaseEvent,
  FtsHit,
  GraphDirection,
  JsonEntry,
  JsonValue,
  KvEntry,
  MutationResult,
  SqlQueryResult,
  WatchSubscription,
} from "./types.js";
import { LiliaError } from "./types.js";
import { checkedVersion, checkedExpiry, readVersion } from "./version.js";

interface NativeKvEntry { namespace: string; key: Buffer; value: Buffer; version: string; expiresAtMs?: number }
interface NativeFtsHit { collection: string; doc_id?: string; docId?: string; snippet: string; rank: number }
interface NativeSubscriptionHandle { unsubscribe(): void }
interface NativeDatabaseHandle {
  kvGet(namespace: string, key: Buffer): Promise<NativeKvEntry | null>;
  kvScan(namespace: string, after: Buffer | undefined, limit: number): Promise<NativeKvEntry[]>;
  jsonGet(space: string, id: string): Promise<string | null>;
  jsonScan(space: string, after: string | undefined, limit: number): Promise<string>;
  batch(operations: string): Promise<string>;
  integrityCheck(): Promise<boolean>;
  backup(destination: string): Promise<void>;
  sqlQuery(sql: string, paramsJson?: string): Promise<string>;
  graphNeighbors(graph: string, node: string, direction: string | undefined, kind: string | undefined): Promise<string[]>;
  graphReachability(graph: string, startNodes: string[], direction: string | undefined): Promise<string[]>;
  ftsSearch(collection: string, query: string, limit: number | undefined): Promise<NativeFtsHit[]>;
  watch(callback: (event: string) => void, namespace?: string): Promise<NativeSubscriptionHandle>;
  close(timeoutMs?: number): Promise<void>;
}
interface NativeModule { NativeDatabase: { open(options: Record<string, unknown>): Promise<NativeDatabaseHandle> } }

const require = createRequire(import.meta.url);

export class EmbeddedClient {
  private constructor(private readonly native: NativeDatabaseHandle) {}

  static async open(options: Record<string, unknown>): Promise<EmbeddedClient> {
    const module = loadNative();
    return new EmbeddedClient(await nativeCall(module.NativeDatabase.open(options)));
  }

  async kvGet(namespace: string, key: Uint8Array): Promise<KvEntry | undefined> {
    const entry = await nativeCall(this.native.kvGet(namespace, Buffer.from(key)));
    return entry ? mapKv(entry) : undefined;
  }

  async kvScan(namespace: string, after: Uint8Array | undefined, limit: number): Promise<KvEntry[]> {
    return (await nativeCall(this.native.kvScan(namespace, after ? Buffer.from(after) : undefined, limit))).map(mapKv);
  }

  async jsonGet(space: string, id: string): Promise<JsonEntry | undefined> {
    const entry = await nativeCall(this.native.jsonGet(space, id));
    return entry ? parseJsonEntry(entry) : undefined;
  }

  async jsonScan(space: string, after: string | undefined, limit: number): Promise<JsonEntry[]> {
    return (JSON.parse(await nativeCall(this.native.jsonScan(space, after, limit))) as Array<Record<string, unknown>>).map(mapJson);
  }

  async batch(operations: readonly BatchOperation[]): Promise<MutationResult[]> {
    const wire = operations.map(toNativeWire);
    const result = JSON.parse(await nativeCall(this.native.batch(JSON.stringify(wire)))) as Array<{ version?: string | null; deleted: boolean }>;
    return result.map(item => ({ ...item, version: item.version == null ? undefined : readVersion(item.version) }));
  }

  async sqlQuery(sql: string, params: unknown[] = []): Promise<SqlQueryResult> {
    const raw = await nativeCall(this.native.sqlQuery(sql, JSON.stringify(params)));
    return mapSqlQueryResult(JSON.parse(raw) as Record<string, unknown>);
  }

  async graphNeighbors(graph: string, node: string, direction?: GraphDirection, kind?: string): Promise<string[]> {
    return nativeCall(this.native.graphNeighbors(graph, node, direction, kind));
  }

  async graphReachability(graph: string, startNodes: string[], direction?: GraphDirection): Promise<string[]> {
    return nativeCall(this.native.graphReachability(graph, startNodes, direction));
  }

  async ftsSearch(collection: string, query: string, limit?: number): Promise<FtsHit[]> {
    const hits = await nativeCall(this.native.ftsSearch(collection, query, limit));
    return hits.map(h => ({
      collection: h.collection,
      docId: h.docId ?? h.doc_id ?? "",
      snippet: h.snippet,
      rank: h.rank,
    }));
  }

  integrityCheck(): Promise<boolean> { return nativeCall(this.native.integrityCheck()); }
  backup(destination: string): Promise<void> { return nativeCall(this.native.backup(destination)); }
  async watch(
    namespace: string | undefined,
    callback: (event: DatabaseEvent) => void
  ): Promise<WatchSubscription> {
    const handle = await nativeCall(
      this.native.watch((...args: unknown[]) => {
        const rawJson = args.length > 1 ? args[1] : args[0];
        try {
          const parsed = typeof rawJson === "string" ? JSON.parse(rawJson) : rawJson;
          const event = mapDatabaseEvent(parsed);
          if (event) {
            callback(event);
          }
        } catch {}
      }, namespace)
    );

    const sub = (() => {
      handle.unsubscribe();
    }) as WatchSubscription;
    sub.unsubscribe = () => {
      handle.unsubscribe();
    };
    return sub;
  }
  close(timeoutMs?: number): Promise<void> { return nativeCall(this.native.close(timeoutMs)); }
}

async function nativeCall<T>(operation: Promise<T>): Promise<T> {
  try { return await operation; }
  catch (error) {
    if (error instanceof Error) {
      let shape;
      try { shape = JSON.parse(error.message); } catch { throw error; }
      if (typeof shape?.code === "string" && typeof shape?.message === "string") throw new LiliaError(shape);
    }
    throw error;
  }
}

function loadNative(): NativeModule {
  let explicit = process.env.LILIA_NATIVE_PATH;
  if (!explicit) {
    const candidates = [
      new URL("../../../target/debug/liblilia_node_native.dylib", import.meta.url).pathname,
      new URL("../../../target/debug/liblilia_node_native.so", import.meta.url).pathname,
      new URL("../../../target/debug/lilia_node_native.dll", import.meta.url).pathname,
      new URL("../../../target/release/liblilia_node_native.dylib", import.meta.url).pathname,
      new URL("../../../target/release/liblilia_node_native.so", import.meta.url).pathname,
      new URL("../../../target/release/lilia_node_native.dll", import.meta.url).pathname,
    ];
    for (const candidate of candidates) {
      if (existsSync(candidate)) {
        explicit = candidate;
        break;
      }
    }
  }
  if (explicit) {
    if (!explicit.endsWith(".node")) {
      const nodePath = `${explicit}.node`;
      if (!existsSync(nodePath)) {
        try { copyFileSync(explicit, nodePath); } catch {}
      }
      if (existsSync(nodePath)) {
        explicit = nodePath;
      }
    }
    return require(explicit) as NativeModule;
  }
  const platform = nativePackagePlatform();
  try { return require(`@liliadb/node-${platform}`) as NativeModule; }
  catch (error) { throw new Error(`No LiliaDB native build for ${platform}; set LILIA_NATIVE_PATH`, { cause: error }); }
}

function nativePackagePlatform(): string {
  if (process.platform === "darwin") return `darwin-${process.arch}`;
  if (process.platform === "linux") return `linux-${process.arch}-gnu`;
  if (process.platform === "win32") return "win32-x64-msvc";
  return `${process.platform}-${process.arch}`;
}

function mapKv(entry: NativeKvEntry): KvEntry {
  return { namespace: entry.namespace, key: entry.key, value: entry.value, version: readVersion(entry.version), expiresAtMs: entry.expiresAtMs };
}
function parseJsonEntry(value: string): JsonEntry { return mapJson(JSON.parse(value) as Record<string, unknown>); }
function mapJson(entry: Record<string, unknown>): JsonEntry {
  return { space: String(entry.space), id: String(entry.id), value: entry.value as JsonEntry["value"], version: readVersion(entry.version) };
}
function toNativeWire(operation: BatchOperation): Record<string, unknown> {
  const result = { ...operation } as Record<string, unknown>;
  if ("ifVersion" in operation) {
    result.if_version = checkedVersion(operation.ifVersion)?.toString();
    delete result.ifVersion;
  }
  if ("expiresAtMs" in operation) {
    result.expires_at_ms = checkedExpiry(operation.expiresAtMs);
    delete result.expiresAtMs;
  }
  if (operation.model === "kv_set" || operation.model === "kv_delete") {
    result.key = [...operation.key];
    if (operation.model === "kv_set") result.value = [...operation.value];
  }
  if (operation.model === "graph_set_node") {
    result.metadata = operation.metadata ?? {};
  }
  if (operation.model === "fts_index" || operation.model === "fts_delete") {
    result.doc_id = operation.docId;
    delete result.docId;
  }
  return result;
}

export function mapSqlQueryResult<T = Record<string, unknown>>(raw: Record<string, unknown>): SqlQueryResult<T> {
  const columns = (raw.columns as string[]) ?? [];
  const rows = (raw.rows as JsonValue[][]) ?? [];
  const rowsAffected = Number(raw.rows_affected ?? 0);
  const records: T[] = rows.map(row => {
    const obj: Record<string, unknown> = {};
    for (let i = 0; i < columns.length; i++) {
      const col = columns[i];
      if (col !== undefined) {
        obj[col] = row[i];
      }
    }
    return obj as T;
  });
  return { columns, rows, rowsAffected, records };
}

export function mapDatabaseEvent(raw: unknown): DatabaseEvent | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const item = raw as Record<string, unknown>;
  const type = String(item.type);
  if (type === "kv_mutated") {
    const rawKey = item.key;
    const key =
      rawKey instanceof Uint8Array
        ? rawKey
        : Array.isArray(rawKey)
        ? new Uint8Array(rawKey)
        : typeof rawKey === "string"
        ? Buffer.from(rawKey, "hex")
        : new Uint8Array();
    return {
      type: "kv_mutated",
      namespace: String(item.namespace),
      key,
      version: item.version == null ? undefined : readVersion(item.version),
      deleted: Boolean(item.deleted),
    };
  }
  if (type === "json_mutated") {
    return {
      type: "json_mutated",
      space: String(item.space),
      id: String(item.id),
      version: item.version == null ? undefined : readVersion(item.version),
      deleted: Boolean(item.deleted),
    };
  }
  if (type === "graph_node_mutated") {
    return {
      type: "graph_node_mutated",
      graph: String(item.graph),
      id: String(item.id),
      deleted: Boolean(item.deleted),
    };
  }
  if (type === "graph_edge_mutated") {
    return {
      type: "graph_edge_mutated",
      graph: String(item.graph),
      source: String(item.source),
      target: String(item.target),
      kind: String(item.kind),
      deleted: Boolean(item.deleted),
    };
  }
  if (type === "fts_mutated") {
    return {
      type: "fts_mutated",
      collection: String(item.collection),
      docId: String(item.doc_id ?? item.docId),
      deleted: Boolean(item.deleted),
    };
  }
  return undefined;
}
