import { DaemonClient } from "./daemon.js";
import { EmbeddedClient } from "./native.js";
import type { BatchOperation, JsonEntry, KvEntry, MutationResult, OpenOptions } from "./types.js";

export * from "./types.js";

type Client = EmbeddedClient | DaemonClient;

export class Database {
  private closed = false;
  private constructor(private readonly client: Client) {}

  static async open(options: OpenOptions): Promise<Database> {
    const plugins = options.plugins ?? ["kv", "json"];
    if (!plugins.includes("kv") || !plugins.includes("json")) throw new Error("the v0.1 distribution requires kv and json plugins");
    const client = options.mode === "daemon"
      ? await DaemonClient.open(options.endpoint ?? `${options.path}.sock`, options.tokenFile ?? `${options.path}.token`)
      : await EmbeddedClient.open(options.path);
    return new Database(client);
  }

  readonly kv = {
    get: (namespace: string, key: Uint8Array): Promise<KvEntry | undefined> => this.active().kvGet(namespace, key),
    set: async (namespace: string, key: Uint8Array, value: Uint8Array, options: { ifVersion?: bigint; expiresAtMs?: number } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "kv_set", namespace, key, value, ...options }]))[0]!,
    delete: async (namespace: string, key: Uint8Array, options: { ifVersion?: bigint } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "kv_delete", namespace, key, ...options }]))[0]!,
    scan: (namespace: string, options: { after?: Uint8Array; limit?: number } = {}): Promise<KvEntry[]> =>
      this.active().kvScan(namespace, options.after, options.limit ?? 100),
  };

  readonly json = {
    get: (space: string, id: string): Promise<JsonEntry | undefined> => this.active().jsonGet(space, id),
    put: async (space: string, id: string, value: JsonEntry["value"], options: { ifVersion?: bigint } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "json_put", space, id, value, ...options }]))[0]!,
    delete: async (space: string, id: string, options: { ifVersion?: bigint } = {}): Promise<MutationResult> =>
      (await this.batch([{ model: "json_delete", space, id, ...options }]))[0]!,
    scan: (space: string, options: { after?: string; limit?: number } = {}): Promise<JsonEntry[]> =>
      this.active().jsonScan(space, options.after, options.limit ?? 100),
  };

  batch(operations: readonly BatchOperation[]): Promise<MutationResult[]> { return this.active().batch(operations); }
  integrityCheck(): Promise<boolean> { return this.active().integrityCheck(); }
  async close(): Promise<void> { if (!this.closed) await this.client.close(); this.closed = true; }
  private active(): Client { if (this.closed) throw new Error("database is closed"); return this.client; }
}
