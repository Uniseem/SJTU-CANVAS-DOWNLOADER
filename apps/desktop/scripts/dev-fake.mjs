#!/usr/bin/env node
// Development against the engine's demo school: no SJTU account is needed.
// Starts the local media server and `electron-vite dev` with the engine's
// test-mode variables. Needs a debug engine (cargo build in engine/).
import { spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { startFakeMedia } from './fake-media.mjs';

const app = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const { server, origin } = await startFakeMedia({ rate: 6_000_000 });
console.log(`fake media at ${origin}`);

const env = {
  ...process.env,
  SJTU_CANVAS_TEST_MODE: '1',
  SJTU_CANVAS_FAKE_SCHOOL: '1',
  SJTU_CANVAS_FAKE_MEDIA: origin,
  SJTU_CANVAS_FAKE_LOGIN_SECONDS: process.env.SJTU_CANVAS_FAKE_LOGIN_SECONDS ?? '3',
  SJTU_CANVAS_DATA_DIR: process.env.SJTU_CANVAS_DATA_DIR ?? mkdtempSync(join(tmpdir(), 'sjtu-canvas-dev-')),
};
console.log(`data in ${env.SJTU_CANVAS_DATA_DIR}`);

const child = spawn(process.platform === 'win32' ? 'npx.cmd' : 'npx', ['electron-vite', 'dev'], {
  cwd: app,
  env,
  stdio: 'inherit',
  shell: process.platform === 'win32',
});
child.on('exit', (code) => {
  server.close();
  process.exit(code ?? 0);
});
