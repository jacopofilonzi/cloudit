// `make dev`: runs the Rust server (cargo run) and the Vite dev server together.
// Output is prefixed per process; when either exits, or on Ctrl+C, both are stopped.
import { spawn, spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const isWindows = process.platform === 'win32';

const processes = [
  { name: 'server', cmd: 'cargo', args: ['run', '-p', 'cloudit-server'] },
  { name: 'web', cmd: 'pnpm', args: ['--filter', '@cloudit/web', 'dev'] }
];

const children = [];
let stopping = false;

function prefixLines(name, stream, out) {
  let pending = '';
  stream.setEncoding('utf8');
  stream.on('data', (chunk) => {
    pending += chunk;
    const lines = pending.split(/\r?\n/);
    pending = lines.pop() ?? '';
    for (const line of lines) out.write(`[${name}] ${line}\n`);
  });
  stream.on('end', () => {
    if (pending) out.write(`[${name}] ${pending}\n`);
  });
}

function kill(child) {
  if (child.exitCode !== null || child.pid === undefined) return;
  if (isWindows) {
    // Kill the whole tree: pnpm/cargo spawn grandchildren that would survive.
    spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
  } else {
    try {
      process.kill(-child.pid, 'SIGTERM');
    } catch {
      child.kill('SIGTERM');
    }
  }
}

function stopAll(code) {
  if (stopping) return;
  stopping = true;
  for (const child of children) kill(child);
  process.exitCode = code;
}

for (const { name, cmd, args } of processes) {
  const child = spawn(cmd, args, {
    cwd: root,
    // `shell` resolves pnpm.cmd on Windows; `detached` gives a process group on Unix.
    shell: isWindows,
    detached: !isWindows,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, FORCE_COLOR: '1' }
  });
  prefixLines(name, child.stdout, process.stdout);
  prefixLines(name, child.stderr, process.stderr);
  child.on('error', (err) => {
    console.error(`[${name}] cannot start ${cmd}: ${err.message}`);
    stopAll(1);
  });
  child.on('exit', (code) => {
    if (!stopping) console.error(`[${name}] exited with code ${code ?? 'null'}, stopping`);
    stopAll(code === 0 ? 0 : 1);
  });
  children.push(child);
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => stopAll(0));
}
