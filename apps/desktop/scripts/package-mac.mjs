#!/usr/bin/env node
// Packages the macOS app: electron-builder builds the .app for this Mac's
// architecture (or --arch arm64|x64), the bundle is signed, and ditto zips it
// as dist/SJTUCanvasDownloader-macos-<arm64|x86_64>.zip, the file install.sh
// downloads. ditto keeps the bundle's symlinks, unlike a plain zip.
//
// Signing: with CSC_LINK (base64 .p12 with a Developer ID Application
// certificate) and CSC_KEY_PASSWORD electron-builder signs with it, and with
// APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID it also notarizes.
// Without a certificate the app is signed ad hoc: it runs after install.sh
// (or a right-click → Open) but shows Gatekeeper's warning when opened from a
// browser download.
import { spawnSync } from 'node:child_process';
import { existsSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'darwin') {
  console.error('package-mac.mjs runs on macOS only');
  process.exit(2);
}

const here = dirname(fileURLToPath(import.meta.url));
const app = resolve(here, '..');
const dist = join(app, 'dist');

let arch = process.arch === 'arm64' ? 'arm64' : 'x64';
const args = process.argv.slice(2);
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === '--arch') {
    arch = args[index + 1] === 'arm64' ? 'arm64' : 'x64';
    index += 1;
  }
}

function run(command, commandArgs, env = {}) {
  console.log(`==> ${command} ${commandArgs.join(' ')}`);
  const result = spawnSync(command, commandArgs, { stdio: 'inherit', cwd: app, env: { ...process.env, ...env } });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

const signed = Boolean(process.env.CSC_LINK);
const notarize = signed && Boolean(process.env.APPLE_ID && process.env.APPLE_APP_SPECIFIC_PASSWORD && process.env.APPLE_TEAM_ID);
const builderArgs = ['electron-builder', '--mac', 'dir', arch === 'arm64' ? '--arm64' : '--x64'];
if (notarize) {
  builderArgs.push('--config.mac.notarize=true');
}
run('npx', builderArgs, signed ? {} : { CSC_IDENTITY_AUTO_DISCOVERY: 'false' });

const folder = join(dist, arch === 'arm64' ? 'mac-arm64' : 'mac');
const bundle = join(folder, 'SJTU Canvas Downloader.app');
if (!existsSync(bundle)) {
  console.error(`missing ${bundle}`);
  process.exit(1);
}
if (!signed) {
  // An ad-hoc signature seals the bundle so macOS never reports it damaged.
  run('codesign', ['--force', '--deep', '--sign', '-', bundle]);
}
run('codesign', ['--verify', '--deep', '--strict', bundle]);

const archive = join(dist, `SJTUCanvasDownloader-macos-${arch === 'arm64' ? 'arm64' : 'x86_64'}.zip`);
rmSync(archive, { force: true });
run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', bundle, archive]);
console.log(`==> ${archive}`);
