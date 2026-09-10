import { copyFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { command } from './process.mjs';

for (const [binary, args] of [
  ['cargo', ['build', '--workspace']],
  [process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm', ['--filter', '@liliadb/node', 'build']],
]) {
  const result = command(binary, args, { stdio: 'inherit', timeout: 0, shell: process.platform === 'win32' && binary === 'pnpm.cmd' });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
const library = process.platform === 'win32' ? 'lilia_node_native.dll'
  : process.platform === 'darwin' ? 'liblilia_node_native.dylib' : 'liblilia_node_native.so';
copyFileSync(resolve('target/debug', library), resolve('target/debug/lilia_node_native.node'));
