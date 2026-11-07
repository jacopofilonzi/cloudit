// `make test-ha`: runs the server tests against PostgreSQL, MinIO (S3) and Valkey.
//
// 1. starts postgres, minio and valkey from docker-compose.ha.yml as a separate
//    compose project (`cloudit-test`, so it never touches the `make up-ha` stack) and
//    creates the bucket with the one-shot minio-init service;
// 2. reads the random host ports Docker assigned (`docker compose port`);
// 3. runs `cargo test -p cloudit-server` with TEST_DATABASE_URL, TEST_S3_* and
//    TEST_REDIS_URL pointing at them (Makefile recipes can't set env inline);
// 4. always removes the project and its volumes, unless `--keep` is passed.
//
// Usage: node scripts/test-ha.mjs [--keep] [-- <extra cargo test args>]
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const compose = ['compose', '-f', 'docker-compose.ha.yml', '-p', 'cloudit-test'];
const args = process.argv.slice(2);
const keep = args.includes('--keep');
const dashdash = args.indexOf('--');
const cargoExtra = dashdash >= 0 ? args.slice(dashdash + 1) : [];

function run(cmd, cmdArgs, options = {}) {
  const result = spawnSync(cmd, cmdArgs, { cwd: root, stdio: 'inherit', ...options });
  if (result.error) throw new Error(`cannot run ${cmd}: ${result.error.message}`);
  return result.status ?? 1;
}

function must(cmd, cmdArgs) {
  const status = run(cmd, cmdArgs);
  if (status !== 0) throw new Error(`${cmd} ${cmdArgs.join(' ')} failed with status ${status}`);
}

function hostPort(service, port) {
  const result = spawnSync('docker', [...compose, 'port', service, String(port)], {
    cwd: root,
    encoding: 'utf8'
  });
  const out = (result.stdout ?? '').trim().split(/\r?\n/)[0] ?? '';
  const match = out.match(/:(\d+)$/);
  if (result.status !== 0 || !match) {
    throw new Error(`cannot find the host port of ${service}:${port} (${out || result.stderr})`);
  }
  return match[1];
}

let status = 1;
try {
  must('docker', [...compose, 'up', '-d', '--wait', 'postgres', 'minio', 'valkey']);
  must('docker', [...compose, 'run', '--rm', 'minio-init']);

  const env = {
    ...process.env,
    TEST_DATABASE_URL: `postgres://cloudit:cloudit-dev-password@127.0.0.1:${hostPort('postgres', 5432)}/cloudit`,
    TEST_S3_ENDPOINT: `http://127.0.0.1:${hostPort('minio', 9000)}`,
    TEST_S3_REGION: 'us-east-1',
    TEST_S3_BUCKET: 'cloudit',
    TEST_S3_ACCESS_KEY_ID: 'cloudit',
    TEST_S3_SECRET_ACCESS_KEY: 'cloudit-dev-secret',
    TEST_REDIS_URL: `redis://127.0.0.1:${hostPort('valkey', 6379)}`
  };
  console.info(
    `test-ha: postgres ${env.TEST_DATABASE_URL.replace(/:[^:@/]*@/, ':***@')}, ` +
      `s3 ${env.TEST_S3_ENDPOINT}, redis ${env.TEST_REDIS_URL}`
  );
  status = run('cargo', ['test', '-p', 'cloudit-server', '--locked', ...cargoExtra], { env });
} catch (err) {
  console.error(`test-ha: ${err.message}`);
  status = 1;
} finally {
  if (keep) {
    console.info('test-ha: --keep: leaving the cloudit-test project running');
  } else {
    run('docker', [...compose, 'down', '-v', '--remove-orphans']);
  }
}
process.exit(status);
