import assert from 'node:assert/strict';
import { mkdtemp, realpath, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { command, bounded } from './process.mjs';
import { cli, daemon, mcp, sdk } from './adapters.mjs';
import { Database } from '../../packages/node/dist/index.js';
import { precision } from './versions.mjs';

const rust = command('cargo', ['test', '-p', 'lilia-storage-sqlite', '--test', 'conformance'], { stdio: 'inherit', timeout: 0 });
assert.equal(rust.status, 0, 'Rust embedded conformance');
const codec = command(process.execPath, ['--test', 'packages/node/dist/version.test.js'], { stdio: 'inherit' });
assert.equal(codec.status, 0, 'SDK version codec validation');
const fixtures = JSON.parse(await readFile(new URL('./fixtures.json', import.meta.url), 'utf8'));
assert(fixtures.length > 0);
process.env.LILIA_NATIVE_PATH = resolve('target/debug/lilia_node_native.node');
const executable = name => resolve('target/debug', name + (process.platform === 'win32' ? '.exe' : ''));
const directory = await realpath(await mkdtemp(join(tmpdir(), 'lilia-conf-')));

async function check(surface, execute) {
  for (const step of fixtures) {
    let result;
    try { result = { value: await bounded(execute(step), `${surface}: ${step.name}`) }; }
    catch (error) { result = { error }; }
    const context = `${surface}: ${step.name}`;
    if (step.error) {
      assert(result.error, `${context}: expected error`);
      assert.deepEqual({ code: result.error.code, retryable: result.error.retryable }, step.error, context);
      assert.match(result.error.requestId ?? result.error.request_id ?? '', /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i, context);
    } else {
      if (result.error) throw new Error(context, { cause: result.error });
      assert.deepEqual(result.value, step.expected, context);
    }
  }
  console.log(`${surface}: ${fixtures.length} shared fixtures passed`);
}

try {
  const embedded = await Database.open({ path: join(directory, 'embedded.lilia') });
  try { await check('Node embedded', sdk(embedded)); }
  finally { await embedded.close(); }
  const shared = await daemon(executable('lilia-daemon'), directory);
  try { await check('daemon via Node SDK', shared.execute); }
  finally { await shared.close(); }
  await check('CLI JSON', cli(executable('lilia'), join(directory, 'cli.lilia'), directory));
  const malformed = command(executable('lilia'), ['--database', join(directory, 'cli.lilia'), '--output', 'json', 'json', 'put', 'docs', 'invalid', '{']);
  assert.equal(malformed.status, 1);
  assert.equal(malformed.stdout, '');
  const parseError = JSON.parse(malformed.stderr).error;
  assert.equal(parseError.code, 'INVALID_INPUT');
  assert.equal(typeof parseError.request_id, 'string');
  const server = await mcp(executable('lilia-mcp'), join(directory, 'mcp.lilia'));
  try {
    await check('MCP stdio', server.execute);
    await assert.rejects(server.execute({ op: 'json_get', space: 'docs', id: 'a', database: 'unknown' }), error => error.code === 'UNAUTHORIZED' && typeof error.request_id === 'string');
    await assert.rejects(server.execute({ op: 'batch', operations: [{ model: 'json_put' }] }), error => error.code === 'INVALID_INPUT' && typeof error.request_id === 'string');
  }
  finally { await server.close(); }
  console.log(`Conformance passed: 5 surfaces × ${fixtures.length} shared steps`);
  await precision(directory, executable);
} finally { await rm(directory, { recursive: true, force: true }); }
