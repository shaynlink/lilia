import { DaemonClient } from "./daemon.js";
import { EmbeddedClient } from "./native.js";
import { randomUUID } from "node:crypto";
import { LiliaError } from "./types.js";
import type { BatchOperation, JsonEntry, KvEntry, MutationResult, OpenOptions } from "./types.js";

export * from "./types.js";

type Client = EmbeddedClient | DaemonClient;

export class Database {
  private closed = false;
  private readonly pending = new Set<Promise<unknown>>();
  private closing?: Promise<void>;
  private constructor(private readonly client: Client) {}

  static async open(options: OpenOptions): Promise<Database> {
    for (const value of [options.checkpointPolicy?.walBytes, options.checkpointPolicy?.intervalMs]) {
      if (value !== undefined && (!Number.isInteger(value) || value < 1 || value > 0xffff_ffff)) {
        throw new RangeError("checkpoint policy values must be integers in 1..=4294967295");
      }
    }
    const plugins = options.plugins ?? ["kv", "json"];
    if (!plugins.includes("kv") || !plugins.includes("json")) throw new Error("the v0.1 distribution requires kv and json plugins");
    const client = options.mode === "daemon"
      ? await DaemonClient.open(options.endpoint ?? `${options.path}.sock`, options.tokenFile ?? `${options.path}.token`)
      : await EmbeddedClient.open({
        path: options.path,
        durability: options.durability,
        busyTimeoutMs: options.busyTimeoutMs,
        writerQueueCapacity: options.writerQueueCapacity,
        readPoolSize: options.readPoolSize,
        checkpointPolicy: options.checkpointPolicy,
      });
    return new Database(client);
  }

  readonly kv = {
    get: (namespace: string, key: Uint8Array): Promise<KvEntry | undefined> => this.run(client => client.kvGet(namespace, key)),
    set: async (namespace: string, key: Uint8Array, value: Uint8Array, options: { ifVersion?: bigint; expiresAtMs?: number } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "kv_set", namespace, key, value, ...options }]))[0]!,
    delete: async (namespace: string, key: Uint8Array, options: { ifVersion?: bigint } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "kv_delete", namespace, key, ...options }]))[0]!,
    scan: (namespace: string, options: { after?: Uint8Array; limit?: number } = {}): Promise<KvEntry[]> =>
      this.run(client => client.kvScan(namespace, options.after, options.limit ?? 100)),
  };

  readonly json = {
    get: (space: string, id: string): Promise<JsonEntry | undefined> => this.run(client => client.jsonGet(space, id)),
    put: async (space: string, id: string, value: JsonEntry["value"], options: { ifVersion?: bigint } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "json_put", space, id, value, ...options }]))[0]!,
    delete: async (space: string, id: string, options: { ifVersion?: bigint } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "json_delete", space, id, ...options }]))[0]!,
    scan: (space: string, options: { after?: string; limit?: number } = {}): Promise<JsonEntry[]> =>
      this.run(client => client.jsonScan(space, options.after, options.limit ?? 100)),
  };

  batch(operations: readonly BatchOperation[]): Promise<MutationResult[]> { return this.run(client => client.batch(operations)); }
  integrityCheck(): Promise<boolean> { return this.run(client => client.integrityCheck()); }
  backup(destination: string): Promise<void> { return this.run(client => client.backup(destination)); }

  async close(options: { timeoutMs?: number } = {}): Promise<void> {
    const timeout = options.timeoutMs ?? 5_000;
    if (!Number.isInteger(timeout) || timeout < 0 || timeout > 0x7fff_ffff) {
      throw new RangeError("close timeoutMs must be an integer in 0..=2147483647");
    }
    if (!this.closing) {
      this.closed = true;
      // Include calls accepted by JS but not yet admitted by a native worker.
      // Caller timeout does not cancel these promises or the eventual cleanup.
      this.closing = Promise.allSettled([...this.pending]).then(() => this.client.close(0xffff_ffff));
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([this.closing, new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(lifecycleError("TIMEOUT", "close is still draining; operations are not cancelled", true)), timeout);
      })]);
    } finally { clearTimeout(timer); }
  }

  private run<T>(action: (client: Client) => Promise<T>): Promise<T> {
    if (this.closed) return Promise.reject(lifecycleError("CLOSED", "database is closing or closed", false));
    const operation = Promise.resolve().then(() => action(this.client));
    this.pending.add(operation);
    void operation.then(() => this.pending.delete(operation), () => this.pending.delete(operation));
    return operation;
  }
}

function lifecycleError(code: string, message: string, retryable: boolean): LiliaError {
  return new LiliaError({ code, message, retryable, request_id: randomUUID() });
}
