import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { command, rpc, service, bounded } from './process.mjs';
import { Database } from '../../packages/node/dist/index.js';

function sdkOperation(operation) {
  const { if_version, expires_at_ms, ...mapped } = operation;
  if (if_version != null) mapped.ifVersion = BigInt(if_version);
  if (expires_at_ms != null) mapped.expiresAtMs = expires_at_ms;
  if ('key' in mapped) mapped.key = Uint8Array.from(mapped.key);
  if (operation.model === 'kv_set') mapped.value = Uint8Array.from(mapped.value);
  return mapped;
}

export function sdk(database) {
  return async step => {
    switch (step.op) {
      case 'batch': return (await database.batch(step.operations.map(sdkOperation)))
        .map(item => {
          if (item.version !== undefined) assert.equal(typeof item.version, 'bigint');
          return { deleted: item.deleted, version: item.version == null ? null : Number(item.version) };
        });
      case 'kv_get': return kv(await database.kv.get(step.namespace, Uint8Array.from(step.key)));
      case 'kv_scan': return (await database.kv.scan(step.namespace, {
        after: step.after ? Uint8Array.from(step.after) : undefined, limit: step.limit,
      })).map(kv);
      case 'json_get': return json(await database.json.get(step.space, step.id));
      case 'json_scan': return (await database.json.scan(step.space, { after: step.after, limit: step.limit })).map(json);
      default: throw new Error(`unsupported fixture: ${step.op}`);
    }
  };
}

function kv(entry) {
  if (entry != null) {
    assert(entry.key instanceof Uint8Array);
    assert(entry.value instanceof Uint8Array);
    assert.equal(typeof entry.version, 'bigint');
  }
  return entry == null ? null : { namespace: entry.namespace, key: [...entry.key], value: [...entry.value],
    version: Number(entry.version), expires_at_ms: entry.expiresAtMs ?? null };
}
function json(entry) {
  if (entry != null) assert.equal(typeof entry.version, 'bigint');
  return entry == null ? null : { ...entry, version: Number(entry.version) };
}

export function cli(binary, path, directory) {
  return async step => {
    let args;
    if (step.op === 'batch') {
      const file = join(directory, 'batch.json');
      await writeFile(file, JSON.stringify(step.operations));
      args = ['batch', file];
    } else {
      const [model, action] = step.op.split('_');
      args = [model, action, step.namespace ?? step.space];
      // The CLI currently exposes text keys; fixtures deliberately use ASCII keys.
      if (action === 'get') args.push(step.id ?? Buffer.from(step.key).toString('utf8'));
      if (action === 'scan') {
        args.push('--limit', String(step.limit));
        if (step.after != null) args.push('--after', model === 'kv' ? Buffer.from(step.after).toString('utf8') : step.after);
      }
    }
    const result = command(binary, ['--database', path, '--output', 'json', ...args]);
    if (result.status !== 0) {
      assert.equal(result.status, 1);
      assert.equal(result.stdout, '');
      throw JSON.parse(result.stderr).error;
    }
    assert.equal(result.stderr, '');
    return JSON.parse(result.stdout);
  };
}

export async function mcp(binary, path) {
  const server = service(binary, ['--database', `test=${path}`]);
  const call = rpc(server);
  try {
    const initialized = await call('initialize', { protocolVersion: '2026-07-28', capabilities: {}, clientInfo: { name: 'conformance', version: '1' } });
    assert.equal(initialized.result.serverInfo.name, 'liliadb');
    server.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
    const listed = await call('tools/list');
    const names = new Set(listed.result.tools.map(tool => tool.name));
    for (const name of ['batch', 'kv_get', 'kv_scan', 'json_get', 'json_scan']) assert(names.has(`lilia_${name}`));
    return {
      close: () => server.close(),
      async execute(step) {
        const { op, name, expected, error, ...args } = step;
        if (args.key) args.key = Buffer.from(args.key).toString('base64');
        if (op === 'kv_scan' && args.after) args.after = Buffer.from(args.after).toString('base64');
        const response = await call('tools/call', { name: `lilia_${op}`, arguments: { database: 'test', ...args } });
        assert.equal(response.jsonrpc, '2.0');
        assert.equal(response.error, undefined);
        const value = response.result.structuredContent;
        assert.deepEqual(JSON.parse(response.result.content[0].text), value);
        if (response.result.isError) throw value.error;
        return value;
      },
    };
  } catch (error) { await server.close(); throw error; }
}

export async function daemon(binary, directory) {
  const path = join(directory, 'daemon.lilia');
  const endpoint = process.platform === 'win32' ? `\\\\.\\pipe\\lilia-conformance-${process.pid}` : join(directory, 'ipc', 'db.sock');
  const tokenFile = join(directory, 'ipc', 'token');
  const server = service(binary, ['--database', path, '--endpoint', endpoint, '--token-file', tokenFile]);
  try {
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      server.check();
      try {
        const database = await bounded(Database.open({ path, mode: 'daemon', endpoint, tokenFile }), 'daemon handshake');
        return { execute: sdk(database), close: async () => { try { await database.close(); } finally { await server.close(); } } };
      } catch { await new Promise(resolve => setTimeout(resolve, 25)); }
    }
    throw new Error('daemon startup timed out');
  } catch (error) { await server.close(); throw error; }
}
