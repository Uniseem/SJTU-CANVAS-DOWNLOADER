import { readFileSync, renameSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { BrowserWindow, screen } from 'electron';

const FILE = 'window.json';

export interface WindowState {
  width: number;
  height: number;
  x?: number;
  y?: number;
  maximized: boolean;
}

const DEFAULTS: WindowState = { width: 1180, height: 800, maximized: false };

export function loadWindowState(directory: string): WindowState {
  try {
    const saved = JSON.parse(readFileSync(join(directory, FILE), 'utf8')) as Partial<WindowState>;
    const state: WindowState = {
      width: clamp(saved.width, 900, 10_000, DEFAULTS.width),
      height: clamp(saved.height, 600, 10_000, DEFAULTS.height),
      maximized: saved.maximized === true,
    };
    if (typeof saved.x === 'number' && typeof saved.y === 'number') {
      // Only restore a position that is still on a connected display.
      const visible = screen.getAllDisplays().some((display) => {
        const { x, y, width, height } = display.workArea;
        return saved.x! + 100 <= x + width && saved.x! + state.width - 100 >= x && saved.y! >= y - 8 && saved.y! + 100 <= y + height;
      });
      if (visible) {
        state.x = Math.round(saved.x);
        state.y = Math.round(saved.y);
      }
    }
    return state;
  } catch {
    return { ...DEFAULTS };
  }
}

export function saveWindowState(directory: string, window: BrowserWindow): void {
  try {
    const maximized = window.isMaximized();
    const bounds = maximized ? window.getNormalBounds() : window.getBounds();
    const state: WindowState = { width: bounds.width, height: bounds.height, x: bounds.x, y: bounds.y, maximized };
    const path = join(directory, FILE);
    writeFileSync(`${path}.tmp`, JSON.stringify(state));
    renameSync(`${path}.tmp`, path);
  } catch {
    // Window placement is best-effort.
  }
}

function clamp(value: unknown, min: number, max: number, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? Math.min(max, Math.max(min, Math.round(value))) : fallback;
}
