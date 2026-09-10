import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { join } from 'node:path';
import { Database } from '../../packages/node/dist/index.js';
import { sdk, daemon, cli, mcp } from './adapters.mjs';
import { command, bounded } from './process.mjs';

const high = 9007199254740993n;
const maximum = 9223372036854775807n;
const u64Max = 18446744073709551615n;
const batch = (id, version) => ({ op: 'batch', operations: [
  { model: 'kv_set', namespace: 'versions', key: [...Buffer.from(id)], value: [43], if_version: version.toString(), expires_at_ms: 4102444800000 },
  { model: 'json_put', space: 'versions', id, value: { number: 4294967297 }, if_version: version.toString() },
] });

async function verify(surface, execute) {
  const call = async step => {
    try { return await bounded(execute(step), `${surface} precision ${step.op}`); }
    catch (error) {
      if (error instanceof Error) error.message = `${surface} ${step.op}: ${error.message}`;
      throw error;
    }
  };
  const getKv = id => call({ op: 'kv_get', namespace: 'versions', key: [...Buffer.from(id)] });
  const getJson = id => call({ op: 'json_get', space: 'versions', id });
  const rejected = (step, code, details) => assert.rejects(call(step), error => {
    assert.equal(error.code, code, `${surface}: error code`);
    assert.equal(error.retryable, code === 'CONFLICT');
    assert.equal(typeof (error.requestId ?? error.request_id), 'string');
    if (details) assert.deepEqual(error.details, details, `${surface}: exact conflict details`);
    return true;
  });
  assert.equal((await getKv('high')).version, high.toString());
  const document = await getJson('high');
  assert.equal(document.version, high.toString());
  assert.deepEqual(document.value, { number: 4294967297 });
  assert.equal((await getKv('high')).expires_at_ms, 4102444800000);
  assert.deepEqual(await call(batch('high', high)), [
    { version: (high + 1n).toString(), deleted: false }, { version: (high + 1n).toString(), deleted: false },
  ]);
  for (const expected of [high, u64Max]) {
    await rejected(batch('high', expected), 'CONFLICT', { expected: expected.toString(), actual: (high + 1n).toString() });
  }
  const rollback = batch('high', high);
  rollback.operations.unshift({ model: 'json_put', space: 'versions', id: 'rollback', value: true });
  await rejected(rollback, 'CONFLICT');
  assert.equal(await getJson('rollback'), null);
  assert.equal((await getKv('high')).version, (high + 1n).toString());
  for (const op of ['kv_scan', 'json_scan']) {
    const page = await call({ op, namespace: 'versions', space: 'versions', after: op === 'kv_scan' ? [...Buffer.from('edge')] : 'edge', limit: 1 });
    assert.equal(page[0].version, (high + 1n).toString());
  }
  assert.deepEqual(await call(batch('edge', maximum - 1n)), [
    { version: maximum.toString(), deleted: false }, { version: maximum.toString(), deleted: false },
  ]);
  await rejected(batch('edge', maximum), 'STORAGE');
  assert.equal((await getJson('edge')).version, maximum.toString());
  // JSON/native parsing and SDK range checks must reject, never wrap or round.
  for (const version of [-1n, u64Max + 1n]) await rejected(batch('high', version), 'INVALID_INPUT');
  assert.equal((await getJson('high')).version, (high + 1n).toString());
  assert.deepEqual(await call({ op: 'batch', operations: [
    { model: 'kv_delete', namespace: 'versions', key: [...Buffer.from('high')], if_version: (high + 1n).toString() },
    { model: 'json_delete', space: 'versions', id: 'high', if_version: (high + 1n).toString() },
  ] }), [{ version: null, deleted: true }, { version: null, deleted: true }]);
  assert.equal(await getKv('high'), null);
  assert.equal(await getJson('high'), null);
  console.log(`${surface}: exact versions >2^53, u64::MAX conflicts, i64::MAX exhaustion passed`);
}

export async function precision(parent, executable) {
  const directory = await mkdtemp(join(parent, 'precision-'));
  const result = command('cargo', ['test', '-p', 'lilia-storage-sqlite', '--test', 'versions', '--', '--include-ignored'], {
    stdio: 'inherit', timeout: 0, env: { ...process.env, LILIA_VERSION_FIXTURE_DIR: directory },
  });
  assert.equal(result.status, 0, 'seed version fixtures and validate Rust embedded');
  const embedded = await Database.open({ path: join(directory, 'embedded.lilia') });
  try { await verify('Node embedded', sdk(embedded)); }
  finally { await embedded.close(); }
  const shared = await daemon(executable('lilia-daemon'), directory);
  try { await verify('daemon', shared.execute); }
  finally { await shared.close(); }
  await verify('CLI JSON', cli(executable('lilia'), join(directory, 'cli.lilia'), directory));
  const server = await mcp(executable('lilia-mcp'), join(directory, 'mcp.lilia'));
  try { await verify('MCP stdio', server.execute); }
  finally { await server.close(); }
}
