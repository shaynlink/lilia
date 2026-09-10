import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline';

export function command(binary, args, options = {}) {
  const result = spawnSync(binary, args, { encoding: 'utf8', timeout: 60_000, ...options });
  if (result.error) throw result.error;
  if (result.signal) throw new Error(`${binary} terminated: ${result.signal}`);
  return result;
}

export function service(binary, args) {
  const child = spawn(binary, args, { stdio: ['pipe', 'pipe', 'pipe'] });
  let diagnostic = '';
  child.stderr.on('data', chunk => { diagnostic = (diagnostic + chunk).slice(-8192); });
  const exited = new Promise(resolve => {
    child.once('error', error => resolve({ error }));
    child.once('close', code => resolve({ code }));
  });
  let failure;
  void exited.then(result => { failure = result; });
  return {
    child, exited,
    check() { if (failure) throw new Error(`service exited: ${JSON.stringify(failure)} ${diagnostic}`); },
    async close() {
      if (!failure) child.kill();
      await bounded(exited, 'service shutdown');
    },
  };
}

export async function bounded(promise, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} timed out`)), 15_000);
    })]);
  } finally { clearTimeout(timer); }
}

export function rpc(server) {
  let nextId = 0;
  const pending = new Map();
  const lines = createInterface({ input: server.child.stdout });
  lines.on('line', line => {
    try {
      const response = JSON.parse(line);
      const waiter = pending.get(response.id);
      if (!waiter) throw new Error('unexpected MCP response ID');
      pending.delete(response.id);
      waiter.resolve(response);
    } catch (error) {
      for (const waiter of pending.values()) waiter.reject(error);
      pending.clear();
    }
  });
  void server.exited.then(() => {
    for (const waiter of pending.values()) waiter.reject(new Error('MCP exited before response'));
    pending.clear();
  });
  return async (method, params) => {
    server.check();
    const id = ++nextId;
    const response = new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); });
    server.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    try { return await bounded(response, method); }
    finally { pending.delete(id); }
  };
}
