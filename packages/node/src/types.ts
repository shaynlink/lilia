export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export type DatabaseMode = "embedded" | "daemon";
export type Durability = "durable" | "balanced" | "performance";

export interface OpenOptions {
  path: string;
  mode?: DatabaseMode;
  durability?: Durability;
  plugins?: readonly ("kv" | "json")[];
  endpoint?: string;
  tokenFile?: string;
  busyTimeoutMs?: number;
  writerQueueCapacity?: number;
  readPoolSize?: number;
  checkpointPolicy?: { walBytes?: number; intervalMs?: number };
  encryptionKey?: string;
}

export interface KvEntry {
  namespace: string;
  key: Uint8Array;
  value: Uint8Array;
  version: bigint;
  expiresAtMs?: number;
}

export interface JsonEntry {
  space: string;
  id: string;
  value: JsonValue;
  version: bigint;
}

export interface JsonIndexInfo {
  space: string;
  indexName: string;
  jsonPath: string;
}

export interface CreateIndexOptions {
  name?: string;
  path?: string;
}

export interface JsonFindOptions {
  after?: string;
  limit?: number;
}


export interface ScanStreamOptions<T = string | Uint8Array> {
  after?: T;
  limit?: number;
  batchSize?: number;
}

export type GraphDirection = "outgoing" | "incoming" | "both";

export interface FtsHit {
  collection: string;
  docId: string;
  snippet: string;
  rank: number;
}

export type BatchOperation =
  | { model: "kv_set"; namespace: string; key: Uint8Array; value: Uint8Array; ifVersion?: bigint; expiresAtMs?: number }
  | { model: "kv_delete"; namespace: string; key: Uint8Array; ifVersion?: bigint }
  | { model: "json_put"; space: string; id: string; value: JsonValue; ifVersion?: bigint }
  | { model: "json_delete"; space: string; id: string; ifVersion?: bigint }
  | { model: "graph_set_node"; graph: string; id: string; kind: string; metadata?: JsonValue }
  | { model: "graph_delete_node"; graph: string; id: string }
  | { model: "graph_add_edge"; graph: string; source: string; target: string; kind: string }
  | { model: "graph_remove_edge"; graph: string; source: string; target: string; kind: string }
  | { model: "fts_index"; collection: string; docId: string; content: string }
  | { model: "fts_delete"; collection: string; docId: string };

export type DatabaseEvent =
  | { type: "kv_mutated"; namespace: string; key: Uint8Array; version?: bigint; deleted: boolean }
  | { type: "json_mutated"; space: string; id: string; version?: bigint; deleted: boolean }
  | { type: "graph_node_mutated"; graph: string; id: string; deleted: boolean }
  | { type: "graph_edge_mutated"; graph: string; source: string; target: string; kind: string; deleted: boolean }
  | { type: "fts_mutated"; collection: string; docId: string; deleted: boolean };

export interface WatchSubscription {
  unsubscribe(): void | Promise<void>;
  (): void | Promise<void>;
}

export interface MutationResult { version?: bigint; deleted: boolean }

export interface SqlQueryResult<T = Record<string, unknown>> {
  columns: string[];
  rows: JsonValue[][];
  rowsAffected: number;
  records: T[];
}

export interface LiliaErrorShape {
  code: string;
  message: string;
  retryable: boolean;
  details?: JsonValue;
  request_id: string;
}

export class LiliaError extends Error {
  readonly code: string;
  readonly retryable: boolean;
  readonly details?: JsonValue;
  readonly requestId?: string;

  constructor(shape: Partial<LiliaErrorShape> & { message: string }) {
    super(shape.message);
    this.name = "LiliaError";
    this.code = shape.code ?? "UNKNOWN";
    this.retryable = shape.retryable ?? false;
    this.details = shape.details;
    this.requestId = shape.request_id;
  }
}
