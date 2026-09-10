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

export type BatchOperation =
  | { model: "kv_set"; namespace: string; key: Uint8Array; value: Uint8Array; ifVersion?: bigint; expiresAtMs?: number }
  | { model: "kv_delete"; namespace: string; key: Uint8Array; ifVersion?: bigint }
  | { model: "json_put"; space: string; id: string; value: JsonValue; ifVersion?: bigint }
  | { model: "json_delete"; space: string; id: string; ifVersion?: bigint };

export interface MutationResult { version?: bigint; deleted: boolean }

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
