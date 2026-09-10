import { readFile } from "node:fs/promises";
import { connect, type Socket } from "node:net";
import { randomUUID } from "node:crypto";
import { decode, encode } from "@msgpack/msgpack";
import { LiliaError, type BatchOperation, type JsonEntry, type KvEntry, type MutationResult } from "./types.js";

interface WireResponse { id: string; result: { Ok?: unknown; Err?: { message: string; code: string; retryable: boolean; request_id: string | Uint8Array } } }

export class DaemonClient {
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
  async batch(operations: readonly BatchOperation[]): Promise<MutationResult[]> {
    const result = await this.call({ type: "batch", operations: operations.map(toWire) }) as Array<{ version?: number | null; deleted: boolean }>;
    return result.map(item => ({ ...item, version: item.version == null ? undefined : BigInt(item.version) }));
  }
  async integrityCheck(): Promise<boolean> { return Boolean(await this.call({ type: "integrity_check" })); }
  async backup(destination: string): Promise<void> { await this.call({ type: "backup", destination }); }
  close(_timeoutMs?: number): Promise<void> { return Promise.resolve(); }

  private async call(operation: Record<string, unknown>): Promise<unknown> {
    const socket = connect(this.endpoint);
    await new Promise<void>((resolve, reject) => socket.once("connect", resolve).once("error", reject));
    const payload = encode({ id: randomUUID(), token: this.token, deadline_ms: 5_000, operation });
    const header = Buffer.allocUnsafe(4);
    header.writeUInt32BE(payload.byteLength);
    socket.end(Buffer.concat([header, Buffer.from(payload)]));
    const response = await receive(socket);
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
  return decode(data.subarray(4)) as WireResponse;
}

function unwrapValue(value: unknown): unknown {
  if (!value || typeof value !== "object") return value;
  const record = value as Record<string, unknown>;
  return "value" in record ? record.value : value;
}
function toWire(operation: BatchOperation): Record<string, unknown> {
  const result = { ...operation } as Record<string, unknown>;
  if ("ifVersion" in operation) { result.if_version = operation.ifVersion === undefined ? undefined : Number(operation.ifVersion); delete result.ifVersion; }
  if ("expiresAtMs" in operation) { result.expires_at_ms = operation.expiresAtMs; delete result.expiresAtMs; }
  return result;
}
function mapKv(value: unknown): KvEntry | undefined {
  if (!value) return undefined;
  const item = value as Record<string, unknown>;
  return { namespace: String(item.namespace), key: item.key as Uint8Array, value: item.value as Uint8Array, version: BigInt(item.version as number), expiresAtMs: item.expires_at_ms as number | undefined };
}
function mapJson(value: unknown): JsonEntry | undefined {
  if (!value) return undefined;
  const item = value as Record<string, unknown>;
  return { space: String(item.space), id: String(item.id), value: item.value as JsonEntry["value"], version: BigInt(item.version as number) };
}
