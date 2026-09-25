#!/usr/bin/env node
// Builds the Rust engine in release mode and copies it to resources/engine/,
// where electron-builder picks it up (see electron-builder.yml).
//
//   node scripts/build-engine.mjs [--target <rust-target>] [--debug]
//
// The target defaults to the host (CARGO_BUILD_TARGET is honoured too).
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const app = resolve(here, '..');
const repo = resolve(app, '..', '..');
const manifest = join(repo, 'engine', 'Cargo.toml');

const args = process.argv.slice(2);
let target = process.env.CARGO_BUILD_TARGET || '';
let profile = 'release';
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === '--target') {
    target = args[index + 1] ?? '';
    index += 1;
  } else if (args[index] === '--debug') {
    profile = 'debug';
  } else {
    console.error(`unknown argument: ${args[index]}`);
    process.exit(2);
  }
}

const cargoArgs = ['build', '--locked', '--manifest-path', manifest];
if (profile === 'release') {
  cargoArgs.push('--release');
}
if (target) {
  cargoArgs.push('--target', target);
}
console.log(`==> cargo ${cargoArgs.join(' ')}`);
const build = spawnSync('cargo', cargoArgs, { stdio: 'inherit', cwd: repo });
if (build.status !== 0) {
  process.exit(build.status ?? 1);
}

const exe = process.platform === 'win32' ? 'sjtu-canvas-engine.exe' : 'sjtu-canvas-engine';
const built = join(repo, 'engine', 'target', ...(target ? [target] : []), profile, exe);
const destination = join(app, 'resources', 'engine', exe);
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(built, destination);
console.log(`==> ${destination} (${(statSync(destination).size / 1024 / 1024).toFixed(1)} MB)`);
