// Starts a complete, throw-away EpiCoin stack for the end-to-end tests:
// a fresh chain and node, a fresh database, the API, and the built front end.
//
// Requires a Postgres reachable at E2E_POSTGRES_URL (default: the one of
// `docker compose up -d postgres`). Everything else lives in a temporary directory.

import { execFileSync, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { createWriteStream, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import pg from 'pg';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../..');
const bin = (name) => join(repo, 'target/debug', name);

const POSTGRES_URL =
  process.env.E2E_POSTGRES_URL ?? 'postgres://epicoin:epicoin@127.0.0.1:5433/epicoin';
const DATABASE = 'epicoin_e2e';
const NODE_PORT = 7461;
const API_PORT = 8191;
const WEB_PORT = 4173;
const WEB_URL = `http://127.0.0.1:${WEB_PORT}`;

const children = [];
let workDir;

function stop(code) {
  for (const child of children) child.kill('SIGTERM');
  if (workDir) rmSync(workDir, { recursive: true, force: true });
  process.exit(code);
}
process.on('SIGTERM', () => stop(0));
process.on('SIGINT', () => stop(0));

function start(name, command, args, env, logFile) {
  const child = spawn(command, args, {
    env: { ...process.env, NO_COLOR: '1', ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const log = createWriteStream(logFile);
  child.stdout.pipe(log);
  child.stderr.pipe(log);
  child.on('exit', (code) => {
    if (code !== null && code !== 0) {
      console.error(`${name} exited with code ${code}; see ${logFile}`);
      console.error(readFileSync(logFile, 'utf8').split('\n').slice(-20).join('\n'));
      stop(1);
    }
  });
  children.push(child);
  return child;
}

async function waitFor(url, what) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // Not up yet.
    }
    await new Promise((done) => setTimeout(done, 150));
  }
  throw new Error(`${what} did not become ready at ${url}`);
}

async function main() {
  console.log('Building the Rust binaries…');
  execFileSync('cargo', ['build', '--quiet', '--bins'], { cwd: repo, stdio: 'inherit' });

  workDir = mkdtempSync(join(tmpdir(), 'epicoin-e2e-'));
  const apiLog = join(workDir, 'api.log');

  // A brand-new database.
  const admin = new pg.Client({ connectionString: POSTGRES_URL });
  await admin.connect();
  await admin.query(`DROP DATABASE IF EXISTS ${DATABASE} WITH (FORCE)`);
  await admin.query(`CREATE DATABASE ${DATABASE}`);
  await admin.end();
  const databaseUrl = new URL(POSTGRES_URL);
  databaseUrl.pathname = `/${DATABASE}`;

  // A brand-new chain.
  execFileSync(bin('epicoin-cli'), ['genesis', '--chain-id', 'epicoin-e2e', '--out-dir', workDir]);
  const keys = Object.fromEntries(
    readFileSync(join(workDir, 'keys.env'), 'utf8')
      .split('\n')
      .filter((line) => line.includes('=') && !line.startsWith('#'))
      .map((line) => line.split('=')),
  );

  start(
    'node',
    bin('epicoin-node'),
    ['--data-dir', join(workDir, 'node'), '--genesis', join(workDir, 'genesis.json'),
      '--listen', `127.0.0.1:${NODE_PORT}`],
    { EPICOIN_VALIDATOR_KEY: keys.EPICOIN_VALIDATOR_KEY, EPICOIN_BLOCK_INTERVAL_MS: '300' },
    join(workDir, 'node.log'),
  );
  await waitFor(`http://127.0.0.1:${NODE_PORT}/health`, 'the node');

  const apiEnv = {
    EPICOIN_DATABASE_URL: databaseUrl.toString(),
    EPICOIN_NODE_URL: `http://127.0.0.1:${NODE_PORT}`,
    EPICOIN_API_LISTEN: `127.0.0.1:${API_PORT}`,
    EPICOIN_PUBLIC_URL: WEB_URL,
    EPICOIN_COOKIE_SECURE: 'false',
    EPICOIN_MAILER: 'console',
    EPICOIN_MASTER_KEY: randomBytes(32).toString('hex'),
    EPICOIN_ADMIN_KEY: keys.EPICOIN_ADMIN_KEY,
    EPICOIN_TREASURY_KEY: keys.EPICOIN_TREASURY_KEY,
    EPICOIN_UPLOAD_DIR: join(workDir, 'uploads'),
    EPICOIN_VALIDATOR_KEY: '',
  };

  // The first administrator, created the documented way.
  const created = execFileSync(
    bin('epicoin-api'),
    ['create-admin', '--email', 'admin@epitech.eu', '--first-name', 'Alice', '--last-name', 'Admin'],
    { env: { ...process.env, NO_COLOR: '1', ...apiEnv }, encoding: 'utf8' },
  );
  const adminActivationUrl = /https?:\/\/\S+#token=[0-9a-f]+/.exec(created)?.[0];
  if (!adminActivationUrl) throw new Error(`no activation link in: ${created}`);

  start('api', bin('epicoin-api'), [], apiEnv, apiLog);
  await waitFor(`http://127.0.0.1:${API_PORT}/api/health`, 'the API');

  // What the tests need to know.
  writeFileSync(join(here, '.stack.json'), JSON.stringify({ adminActivationUrl, apiLog }));

  start(
    'web',
    'npx',
    ['vite', 'preview', '--host', '127.0.0.1'],
    { EPICOIN_API_URL: `http://127.0.0.1:${API_PORT}`, EPICOIN_WEB_PORT: String(WEB_PORT) },
    join(workDir, 'web.log'),
  );
  await waitFor(`${WEB_URL}/api/health`, 'the front end');
  console.log(`EpiCoin end-to-end stack ready at ${WEB_URL}`);
}

main().catch((error) => {
  console.error(error);
  stop(1);
});
