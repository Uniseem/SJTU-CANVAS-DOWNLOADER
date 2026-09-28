#!/usr/bin/env node
// End-to-end smoke test of the built app against the engine's demo school:
// the login QR appears, the demo login completes, a course opens, a recording
// and a course file are downloaded, and the settings save. Screenshots go to
// dist/smoke/.
//
//   npm run build && cargo build --manifest-path ../../engine/Cargo.toml
//   node tests/smoke.mjs [--engine <path>] [--electron <path>]
//
// Needs a debug engine (only it has the test mode) and a display (on Linux
// e.g. xvfb-run).
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { _electron as electron } from 'playwright-core';

import { startFakeMedia } from '../scripts/fake-media.mjs';

const app = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(app, '..', '..');
const exe = process.platform === 'win32' ? 'sjtu-canvas-engine.exe' : 'sjtu-canvas-engine';

const args = process.argv.slice(2);
const option = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const enginePath = option('--engine') ?? join(repo, 'engine', 'target', 'debug', exe);
if (!existsSync(enginePath)) {
  console.error(`missing debug engine at ${enginePath}; run: cargo build --manifest-path engine/Cargo.toml`);
  process.exit(2);
}
if (!existsSync(join(app, 'out', 'main', 'index.js'))) {
  console.error('missing out/main/index.js; run: npm run build');
  process.exit(2);
}

const shots = join(app, 'dist', 'smoke');
rmSync(shots, { recursive: true, force: true });
mkdirSync(shots, { recursive: true });
const dataDir = mkdtempSync(join(tmpdir(), 'sjtu-canvas-smoke-'));
const downloads = join(dataDir, 'Downloads');
const { server, origin } = await startFakeMedia({ rate: 8_000_000 });

const checks = [];
function expect(condition, message) {
  if (!condition) {
    throw new Error(`check failed: ${message}`);
  }
  checks.push(message);
  console.log(`  ok  ${message}`);
}

let electronApp;
try {
  electronApp = await electron.launch({
    args: [join(app, 'out', 'main', 'index.js'), '--no-sandbox'],
    executablePath: option('--electron'),
    env: {
      ...process.env,
      SJTU_CANVAS_TEST_MODE: '1',
      SJTU_CANVAS_FAKE_SCHOOL: '1',
      SJTU_CANVAS_FAKE_MEDIA: origin,
      SJTU_CANVAS_FAKE_LOGIN_SECONDS: '1',
      SJTU_CANVAS_DATA_DIR: dataDir,
      SJTU_CANVAS_ENGINE: enginePath,
      SJTU_CANVAS_SESSION_KEY: Buffer.from(Array.from({ length: 32 }, (_, i) => i * 7 + 1)).toString('base64'),
    },
    timeout: 60_000,
  });
  const window = await electronApp.firstWindow({ timeout: 60_000 });
  window.on('pageerror', (error) => console.error('page error:', error));
  window.on('console', (message) => {
    if (message.type() === 'error') {
      console.error('console error:', message.text());
    }
  });
  await window.waitForLoadState('domcontentloaded');

  console.log('login');
  const qr = window.locator('img[alt="交我办登录二维码"]');
  await qr.waitFor({ state: 'visible', timeout: 30_000 });
  expect(await qr.getAttribute('src').then((src) => src?.startsWith('data:image/png;base64,')), 'the QR code is shown');
  await window.screenshot({ path: join(shots, '1-login.png') });

  console.log('courses');
  const coursesHeading = window.getByRole('heading', { level: 1, name: '课程' });
  await coursesHeading.waitFor({ state: 'visible', timeout: 30_000 });
  const course = window.getByRole('button', { name: /机器学习与数据挖掘/ }).first();
  await course.waitFor({ state: 'visible', timeout: 30_000 });
  expect(true, 'the demo courses are listed after the login');
  await window.screenshot({ path: join(shots, '2-courses.png') });

  console.log('course');
  await course.click();
  await window.getByRole('heading', { level: 1, name: /机器学习与数据挖掘/ }).waitFor({ timeout: 30_000 });
  const firstLesson = window.getByRole('checkbox', { name: /^选择 第 01 讲/ });
  await firstLesson.waitFor({ state: 'attached', timeout: 30_000 });
  // The native checkbox is visually hidden; clicking the row toggles it.
  await window.getByText(/^第 01 讲/).first().click();
  expect(await firstLesson.isChecked(), 'clicking a lesson row selects it');
  await window.getByText(/已选 1 讲/).waitFor({ timeout: 10_000 });
  await window.getByText(/约 .* (MB|GB)/).waitFor({ timeout: 30_000 });
  expect(true, 'a selected lesson shows its size');
  await window.screenshot({ path: join(shots, '3-course.png') });
  const downloadButton = window.getByRole('main').getByRole('button', { name: '下载', exact: true });
  await downloadButton.click();
  await window.getByText(/已添加 2 个下载任务/).waitFor({ timeout: 15_000 });
  expect(true, 'two video downloads (two tracks) were queued');

  await window.getByRole('tab', { name: /课程文件/ }).click();
  const firstFile = window.getByRole('checkbox', { name: /^选择 / }).first();
  await firstFile.waitFor({ state: 'attached', timeout: 30_000 });
  await window.locator('span[title]').first().click();
  expect(await firstFile.isChecked(), 'clicking a file row selects it');
  await downloadButton.click();
  await window.getByText(/已添加 1 个下载任务/).waitFor({ timeout: 15_000 });
  expect(true, 'a course file download was queued');

  console.log('downloads');
  await window.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: /^下载/ }).click();
  await window.getByRole('heading', { level: 1, name: '下载' }).waitFor({ timeout: 10_000 });
  await window.screenshot({ path: join(shots, '4-downloads.png') });
  await window.locator('text=已完成').first().waitFor({ timeout: 120_000 });
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    const completed = await window.getByText('已完成', { exact: true }).count();
    if (completed >= 3) {
      break;
    }
    await window.waitForTimeout(500);
  }
  expect((await window.getByText('已完成', { exact: true }).count()) >= 3, 'all three downloads completed');
  await window.screenshot({ path: join(shots, '5-downloads-done.png') });
  const listed = readFileSync(join(dataDir, 'logs', 'app.log'), 'utf8');
  expect(listed.includes('engine') && listed.includes('ready'), 'the host logged the engine start');
  expect(existsSync(downloads) || existsSync(join(dataDir, 'downloads.db')), 'the engine kept its database in the data folder');

  console.log('settings');
  await window.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: '设置' }).click();
  await window.getByRole('heading', { level: 1, name: '设置' }).waitFor({ timeout: 10_000 });
  // The native radio is visually hidden; its label toggles it.
  await window.getByText('不使用代理', { exact: true }).click();
  await window.waitForTimeout(800);
  expect(await window.getByRole('radio', { name: '不使用代理' }).isChecked(), 'the proxy setting changed');
  await window.screenshot({ path: join(shots, '6-settings.png') });

  await electronApp.close();
  electronApp = null;
  console.log(`smoke: ${checks.length} checks passed`);
} catch (error) {
  console.error(error);
  if (electronApp) {
    try {
      const window = await electronApp.firstWindow({ timeout: 5_000 });
      await window.screenshot({ path: join(shots, 'failure.png') });
    } catch {
      // No window to capture.
    }
    await electronApp.close().catch(() => undefined);
  }
  process.exitCode = 1;
} finally {
  server.close();
  if (!process.exitCode) {
    rmSync(dataDir, { recursive: true, force: true });
  } else {
    console.error(`data kept in ${dataDir}`);
  }
}
