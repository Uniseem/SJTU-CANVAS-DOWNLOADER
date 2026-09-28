import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';

import { app } from 'electron';

export const APP_NAME = 'SJTU Canvas Downloader';
export const APP_ID = 'io.github.uniseem.sjtu-canvas-downloader';
export const ENGINE_FILE = process.platform === 'win32' ? 'sjtu-canvas-engine.exe' : 'sjtu-canvas-engine';

/**
 * The engine's data folder: the download list, settings, the saved login and
 * the logs. It is the same folder the engine picks without `--data-dir`, so
 * data of earlier versions is found. SJTU_CANVAS_DATA_DIR overrides it for
 * development and tests.
 */
export function resolveDataDir(): string {
  const override = process.env.SJTU_CANVAS_DATA_DIR?.trim();
  if (override) {
    return override;
  }
  const home = app.getPath('home');
  if (process.platform === 'win32') {
    return join(process.env.LOCALAPPDATA || join(home, 'AppData', 'Local'), APP_NAME);
  }
  if (process.platform === 'darwin') {
    return join(home, 'Library', 'Application Support', APP_NAME);
  }
  return join(process.env.XDG_DATA_HOME || join(home, '.local', 'share'), APP_NAME);
}

/**
 * The engine binary. Installed: <resources>/engine/sjtu-canvas-engine[.exe].
 * During development the repository's cargo build output is used, or
 * SJTU_CANVAS_ENGINE.
 */
export function locateEngine(): string {
  const candidates: string[] = [];
  const override = process.env.SJTU_CANVAS_ENGINE?.trim();
  if (override) {
    candidates.push(override);
  }
  candidates.push(join(process.resourcesPath, 'engine', ENGINE_FILE), join(process.resourcesPath, ENGINE_FILE));
  if (!app.isPackaged) {
    const repo = findRepoRoot(app.getAppPath());
    if (repo) {
      candidates.push(join(repo, 'apps', 'desktop', 'resources', 'engine', ENGINE_FILE));
      // A development app uses the debug engine first: only it has the test mode.
      for (const profile of ['debug', 'release']) {
        candidates.push(join(repo, 'engine', 'target', profile, ENGINE_FILE));
      }
    }
  }
  const found = candidates.find((path) => existsSync(path));
  if (!found) {
    throw new Error(`找不到下载引擎 ${ENGINE_FILE}，请重新安装应用`);
  }
  return found;
}

function findRepoRoot(start: string): string | null {
  let directory = start;
  for (let depth = 0; depth < 8; depth += 1) {
    if (existsSync(join(directory, 'engine', 'Cargo.toml'))) {
      return directory;
    }
    const parent = dirname(directory);
    if (parent === directory) {
      break;
    }
    directory = parent;
  }
  return null;
}
