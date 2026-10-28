import { readFile } from "node:fs/promises";
import { connect, type Socket } from "node:net";
import { randomUUID } from "node:crypto";
import { decode, encode } from "@msgpack/msgpack";
import {
  LiliaError,
  type BatchOperation,
  type DatabaseEvent,
  type FtsHit,
  type GraphDirection,
  type JsonEntry,
  type JsonIndexInfo,
  type JsonValue,
  type KvEntry,
  type MutationResult,
  type SqlQueryResult,
  type WatchSubscription,
} from "./types.js";
import { checkedVersion, checkedExpiry, readVersion, jsonNumbers } from "./version.js";
import { mapDatabaseEvent, mapSqlQueryResult } from "./native.js";

interface WireResponse { id: string; result: { Ok?: unknown; Err?: { message: string; code: string; retryable: boolean; request_id: string | Uint8Array } } }

export class DaemonClient {
  private activeSocket?: Socket;
  private socketBuffer = Buffer.alloc(0);
  private readonly pendingCalls = new Map<string, { resolve: (resp: WireResponse) => void; reject: (err: Error) => void }>();
  private connecting?: Promise<Socket>;

  private constructor(private readonly endpoint: string, private readonly token: string) {}

  static async open(endpoint: string, tokenFile: string): Promise<DaemonClient> {
    const client = new DaemonClient(endpoint, (await readFile(tokenFile, "utf8")).trim());
    await client.call({ type: "handshake", major: 1, minor: 0 });
    return client;
  }

  async kvGet(namespace: string, key: Uint8Array): Promise<KvEntry | undefined> {
    return mapKv(await this.call({ type: "kv_get", namespace, key })) as KvEntry | undefined;
  }
  async kvScan(namespace: string, after: Uint8Array | undefined, limit: number): Promise<KvEntry[]> {
    return ((await this.call({ type: "kv_scan", namespace, after: after ? [...after] : undefined, limit })) as unknown[]).map(value => mapKv(value) as KvEntry);
  }
  async jsonGet(space: string, id: string): Promise<JsonEntry | undefined> {
    return mapJson(await this.call({ type: "json_get", space, id })) as JsonEntry | undefined;
  }
  async jsonScan(space: string, after: string | undefined, limit: number): Promise<JsonEntry[]> {
    return ((await this.call({ type: "json_scan", space, after, limit })) as unknown[]).map(value => mapJson(value) as JsonEntry);
  }
  async jsonCreateIndex(space: string, name: string, path: string): Promise<void> {
    await this.call({ type: "json_create_index", space, name, path });
  }
  async jsonDropIndex(space: string, name: string): Promise<boolean> {
    return Boolean(await this.call({ type: "json_drop_index", space, name }));
  }
  async jsonListIndexes(space: string): Promise<JsonIndexInfo[]> {
    const list = (await this.call({ type: "json_list_indexes", space })) as Array<{ space: string; index_name?: string; indexName?: string; json_path?: string; jsonPath?: string }>;
    return (list ?? []).map(item => ({
      space: item.space,
      indexName: item.indexName ?? item.index_name ?? "",
      jsonPath: item.jsonPath ?? item.json_path ?? "",
    }));
  }
  async jsonScanByIndex(space: string, indexName: string, value: string, after: string | undefined, limit: number): Promise<JsonEntry[]> {
    const list = (await this.call({ type: "json_scan_by_index", space, index_name: indexName, value, after, limit })) as unknown[];
    return (list ?? []).map(value => mapJson(value) as JsonEntry);
  }
  async jsonFind(space: string, query: Record<string, JsonValue>, after: string | undefined, limit: number): Promise<JsonEntry[]> {
    const list = (await this.call({ type: "json_find", space, query, after, limit })) as unknown[];
    return (list ?? []).map(value => mapJson(value) as JsonEntry);
  }
  async batch(operations: readonly BatchOperation[]): Promise<MutationResult[]> {
    const result = await this.call({ type: "batch", operations: operations.map(toWire) }) as Array<{ version?: bigint | number | null; deleted: boolean }>;
    return result.map(item => ({ ...item, version: item.version == null ? undefined : readVersion(item.version) }));
  }
  async integrityCheck(): Promise<boolean> { return Boolean(await this.call({ type: "integrity_check" })); }
  async backup(destination: string): Promise<void> { await this.call({ type: "backup", destination }); }
  async sqlQuery(sql: string, params: unknown[] = []): Promise<SqlQueryResult> {
    const raw = (await this.call({ type: "sql_query", sql, params })) as Record<string, unknown>;
    return mapSqlQueryResult(raw);
  }
  async graphNeighbors(graph: string, node: string, direction?: GraphDirection, kind?: string): Promise<string[]> {
    return ((await this.call({ type: "graph_neighbors", graph, node, direction: direction ?? "outgoing", kind })) as string[]) ?? [];
  }
  async graphReachability(graph: string, startNodes: string[], direction?: GraphDirection): Promise<string[]> {
    return ((await this.call({ type: "graph_reachability", graph, start_nodes: startNodes, direction: direction ?? "outgoing" })) as string[]) ?? [];
  }
  async ftsSearch(collection: string, query: string, limit?: number): Promise<FtsHit[]> {
    const hits = (await this.call({ type: "fts_search", collection, query, limit: limit ?? 10 })) as Array<Record<string, unknown>>;
    return (hits ?? []).map(h => ({
      collection: String(h.collection),
      docId: String(h.doc_id),
      snippet: String(h.snippet),
      rank: Number(h.rank),
    }));
  }
  async watch(
    namespace: string | undefined,
    callback: (event: DatabaseEvent) => void
  ): Promise<WatchSubscription> {
    const socket = connect(this.endpoint);
    await new Promise<void>((resolve, reject) => socket.once("connect", resolve).once("error", reject));
    const payload = encode(
      { id: randomUUID(), token: this.token, operation: { type: "subscribe", namespace } },
      { useBigInt64: true }
    );
    const header = Buffer.allocUnsafe(4);
    header.writeUInt32BE(payload.byteLength);
    socket.write(Buffer.concat([header, Buffer.from(payload)]));

    let buffer = Buffer.alloc(0);

    socket.on("data", (chunk: Buffer) => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 4) {
        const length = buffer.readUInt32BE(0);
        if (buffer.length < 4 + length) {
          break;
        }
        const frameData = buffer.subarray(4, 4 + length);
        buffer = buffer.subarray(4 + length);
        try {
          const wire = decode(frameData, { useBigInt64: true }) as WireResponse;
          if (wire.result.Err) {
            break;
          }
          if (wire.result.Ok) {
            const rawVal = wire.result.Ok as Record<string, unknown>;
            if (rawVal && typeof rawVal === "object" && rawVal.type === "event" && rawVal.value) {
              const event = mapDatabaseEvent(rawVal.value);
              if (event) {
                callback(event);
              }
            } else if (rawVal && typeof rawVal === "object" && rawVal.type !== "ack") {
              const event = mapDatabaseEvent(rawVal);
              if (event) {
                callback(event);
              }
            }
          }
        } catch {}
      }
    });

    const sub = (() => {
      socket.destroy();
    }) as WatchSubscription;
    sub.unsubscribe = () => {
      socket.destroy();
    };
    return sub;
  }

  close(_timeoutMs?: number): Promise<void> {
    if (this.activeSocket) {
      this.activeSocket.destroy();
      this.activeSocket = undefined;
    }
    for (const pending of this.pendingCalls.values()) {
      pending.reject(
        new LiliaError({
          code: "CLOSED",
          message: "daemon client connection closed",
          retryable: false,
          request_id: randomUUID(),
        })
      );
    }
    this.pendingCalls.clear();
    return Promise.resolve();
  }

  private async getSocket(): Promise<Socket> {
    if (this.activeSocket && !this.activeSocket.destroyed) {
      return this.activeSocket;
    }
    if (this.connecting) return this.connecting;

    this.connecting = new Promise<Socket>((resolve, reject) => {
      const socket = connect(this.endpoint);
      socket.once("connect", () => {
        this.activeSocket = socket;
        this.socketBuffer = Buffer.alloc(0);
        this.connecting = undefined;
        resolve(socket);
      });
      socket.once("error", (err) => {
        this.connecting = undefined;
        reject(err);
      });
      socket.on("data", (chunk: Buffer) => {
        this.socketBuffer = Buffer.concat([this.socketBuffer, chunk]);
        while (this.socketBuffer.length >= 4) {
          const length = this.socketBuffer.readUInt32BE(0);
          if (this.socketBuffer.length < 4 + length) {
            break;
          }
          const frameData = this.socketBuffer.subarray(4, 4 + length);
          this.socketBuffer = this.socketBuffer.subarray(4 + length);
          try {
            const wire = decode(frameData, { useBigInt64: true }) as WireResponse;
            const handler = this.pendingCalls.get(wire.id);
            if (handler) {
              this.pendingCalls.delete(wire.id);
              handler.resolve(wire);
            }
          } catch {
            // Drop unparseable frame
          }
        }
      });
      socket.on("close", () => {
        if (this.activeSocket === socket) {
          this.activeSocket = undefined;
        }
        for (const [id, pending] of this.pendingCalls.entries()) {
          this.pendingCalls.delete(id);
          pending.reject(new Error("socket closed unexpectedly"));
        }
      });
    });

    return this.connecting;
  }

  private async call(operation: Record<string, unknown>): Promise<unknown> {
    const socket = await this.getSocket();
    const id = randomUUID();
    const payload = encode(
      { id, token: this.token, deadline_ms: 5_000, operation },
      { useBigInt64: true }
    );
    const header = Buffer.allocUnsafe(4);
    header.writeUInt32BE(payload.byteLength);

    const response = await new Promise<WireResponse>((resolve, reject) => {
      this.pendingCalls.set(id, { resolve, reject });
      socket.write(Buffer.concat([header, Buffer.from(payload)]), (err) => {
        if (err) {
          this.pendingCalls.delete(id);
          reject(err);
        }
      });
    });

    if (response.result.Err) {
      const error = response.result.Err;
      throw new LiliaError({ ...error, request_id: uuidString(error.request_id) });
    }
    return unwrapValue(response.result.Ok);
  }
}

function uuidString(value: string | Uint8Array): string {
  if (typeof value === "string") return value;
  if (!(value instanceof Uint8Array) || value.length !== 16) throw new Error("invalid daemon error request ID");
  const hex = Buffer.from(value).toString("hex");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

async function receive(socket: Socket): Promise<WireResponse> {
  const chunks: Buffer[] = [];
  for await (const chunk of socket) chunks.push(Buffer.from(chunk));
  const data = Buffer.concat(chunks);
  if (data.length < 4) throw new Error("truncated daemon response");
  const length = data.readUInt32BE(0);
  if (length === 0 || length > 16 * 1024 * 1024 || data.length !== length + 4) throw new Error("invalid daemon frame");
  return decode(data.subarray(4), { useBigInt64: true }) as WireResponse;
}

function unwrapValue(value: unknown): unknown {
  if (!value || typeof value !== "object") return value;
  const record = value as Record<string, unknown>;
  return "value" in record ? record.value : value;
}
function toWire(operation: BatchOperation): Record<string, unknown> {
  const result = { ...operation } as Record<string, unknown>;
  if ("ifVersion" in operation) { result.if_version = checkedVersion(operation.ifVersion); delete result.ifVersion; }
  if ("expiresAtMs" in operation) {
    const expiry = checkedExpiry(operation.expiresAtMs);
    result.expires_at_ms = expiry === undefined ? undefined : BigInt(expiry);
    delete result.expiresAtMs;
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
function mapKv(value: unknown): KvEntry | undefined {
  if (!value) return undefined;
  const item = value as Record<string, unknown>;
  return { namespace: String(item.namespace), key: item.key as Uint8Array, value: item.value as Uint8Array, version: readVersion(item.version), expiresAtMs: item.expires_at_ms == null ? undefined : Number(item.expires_at_ms) };
}
function mapJson(value: unknown): JsonEntry | undefined {
  if (!value) return undefined;
  const item = value as Record<string, unknown>;
  return { space: String(item.space), id: String(item.id), value: jsonNumbers(item.value), version: readVersion(item.version) };
}
