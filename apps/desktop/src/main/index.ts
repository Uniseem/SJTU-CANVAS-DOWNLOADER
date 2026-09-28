import { appendFileSync, mkdirSync, renameSync, statSync } from 'node:fs';
import { join } from 'node:path';

import { app, BrowserWindow, dialog, ipcMain, Menu, nativeTheme, Notification, shell } from 'electron';

import type { AppInfo, CallResult, DownloadInfo, EngineNotification, EngineState } from '@shared/protocol';

import { EngineHost, toErrorShape } from './host';
import { APP_ID, APP_NAME, resolveDataDir } from './paths';
import { loadOrCreateSessionKey } from './session-key';
import { loadWindowState, saveWindowState } from './window-state';

app.setName(APP_NAME);
if (process.platform === 'win32') {
  app.setAppUserModelId(APP_ID);
}

// The engine owns <dataDir>; the app keeps its own files in <dataDir>/host.
const dataDir = resolveDataDir();
const hostDir = join(dataDir, 'host');
const logsDir = join(dataDir, 'logs');
for (const directory of [hostDir, logsDir]) {
  mkdirSync(directory, { recursive: true });
}
app.setPath('userData', hostDir);

const logFile = join(logsDir, 'app.log');
function log(line: string): void {
  try {
    try {
      if (statSync(logFile).size > 2 * 1024 * 1024) {
        renameSync(logFile, `${logFile}.1`);
      }
    } catch {
      // No log yet.
    }
    appendFileSync(logFile, `${new Date().toISOString()} ${line}\n`);
  } catch {
    // Logging is best-effort.
  }
}

let mainWindow: BrowserWindow | null = null;
let host: EngineHost | null = null;
let quitting = false;
const notifiedStatus = new Map<string, string>();

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (mainWindow) {
      if (mainWindow.isMinimized()) {
        mainWindow.restore();
      }
      mainWindow.show();
      mainWindow.focus();
    } else {
      createWindow();
    }
  });
  app.whenReady().then(main);
}

async function main(): Promise<void> {
  installMenu();
  const sessionKey = loadOrCreateSessionKey(hostDir, log);
  host = new EngineHost(dataDir, sessionKey, log);
  host.on('state', (state: EngineState) => broadcast('engine:state', state));
  host.on('notification', (notification: EngineNotification) => {
    broadcast('engine:notification', notification);
    if (notification.method === 'download.changed') {
      maybeNotify(notification.params as DownloadInfo);
    }
  });
  registerIpc(host);
  createWindow();
  await host.start();
}

function createWindow(): void {
  const state = loadWindowState(hostDir);
  const window = new BrowserWindow({
    width: state.width,
    height: state.height,
    x: state.x,
    y: state.y,
    minWidth: 900,
    minHeight: 600,
    show: false,
    autoHideMenuBar: true,
    title: APP_NAME,
    backgroundColor: nativeTheme.shouldUseDarkColors ? '#131316' : '#f5f5f5',
    // The renderer draws its own title bar (TitleBar.tsx). macOS keeps its
    // traffic lights at the left; Windows draws its native buttons over the
    // bar's right end, colored to match the theme (window:setTitleBarOverlay).
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : process.platform === 'win32' ? 'hidden' : 'default',
    titleBarOverlay: process.platform === 'win32' ? titleBarOverlay(nativeTheme.shouldUseDarkColors) : undefined,
    trafficLightPosition: process.platform === 'darwin' ? { x: 16, y: 14 } : undefined,
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false,
      spellcheck: false,
    },
  });
  mainWindow = window;
  if (state.maximized) {
    window.maximize();
  }
  window.once('ready-to-show', () => window.show());
  window.on('close', () => saveWindowState(hostDir, window));
  window.on('closed', () => {
    if (mainWindow === window) {
      mainWindow = null;
    }
  });
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//i.test(url)) {
      void shell.openExternal(url);
    }
    return { action: 'deny' };
  });
  window.webContents.on('will-navigate', (event) => event.preventDefault());
  if (!app.isPackaged) {
    window.webContents.on('before-input-event', (_event, input) => {
      if (input.type === 'keyDown' && input.key === 'F12') {
        window.webContents.toggleDevTools();
      }
    });
  }
  const devUrl = process.env.ELECTRON_RENDERER_URL;
  if (devUrl) {
    void window.loadURL(devUrl);
  } else {
    void window.loadFile(join(__dirname, '../renderer/index.html'));
  }
}

const TITLE_BAR_HEIGHT = 40;

function titleBarOverlay(dark: boolean): Electron.TitleBarOverlay {
  return dark
    ? { color: '#29292b', symbolColor: '#e4e4e7', height: TITLE_BAR_HEIGHT }
    : { color: '#f2f2f3', symbolColor: '#3f3f46', height: TITLE_BAR_HEIGHT };
}

function installMenu(): void {
  if (process.platform === 'darwin') {
    Menu.setApplicationMenu(
      Menu.buildFromTemplate([{ role: 'appMenu' }, { role: 'editMenu' }, { role: 'viewMenu' }, { role: 'windowMenu' }]),
    );
  } else {
    Menu.setApplicationMenu(null);
  }
}

function broadcast(channel: string, payload: unknown): void {
  for (const window of BrowserWindow.getAllWindows()) {
    if (!window.isDestroyed()) {
      window.webContents.send(channel, payload);
    }
  }
}

/** A system notification when a download ends while the app is in the background. */
function maybeNotify(download: DownloadInfo): void {
  const previous = notifiedStatus.get(download.id);
  notifiedStatus.set(download.id, download.status);
  if (previous === download.status || !previous) {
    return;
  }
  if (download.status !== 'completed' && download.status !== 'failed') {
    return;
  }
  if (mainWindow?.isFocused() || !Notification.isSupported()) {
    return;
  }
  const notification = new Notification({
    title: download.status === 'completed' ? '下载完成' : '下载失败',
    body: download.status === 'completed' ? download.display_name : `${download.display_name}：${download.error ?? '未知错误'}`,
    silent: true,
  });
  notification.on('click', () => {
    if (mainWindow) {
      mainWindow.show();
      mainWindow.focus();
    } else {
      createWindow();
    }
  });
  notification.show();
}

function registerIpc(engine: EngineHost): void {
  ipcMain.handle('engine:call', async (_event, method: unknown, params: unknown): Promise<CallResult> => {
    if (typeof method !== 'string' || !/^[a-z]+\.[a-zA-Z]+$/.test(method)) {
      return { ok: false, error: { code: 'invalid_params', message: '无效的方法名' } };
    }
    try {
      return { ok: true, result: await engine.call(method, params ?? {}) };
    } catch (error) {
      return { ok: false, error: toErrorShape(error) };
    }
  });
  ipcMain.handle('engine:state', () => engine.state);
  ipcMain.handle('engine:restart', () => engine.restart());
  ipcMain.handle('app:info', (): AppInfo => ({
    version: app.getVersion(),
    platform: process.platform,
    dataDir,
    logsDir,
    downloadsFolder: app.getPath('downloads'),
  }));
  ipcMain.handle('dialog:chooseFolder', async (event, defaultPath: unknown) => {
    const window = BrowserWindow.fromWebContents(event.sender) ?? undefined;
    const result = await dialog.showOpenDialog(window!, {
      title: '选择保存位置',
      buttonLabel: '选择',
      defaultPath: typeof defaultPath === 'string' && defaultPath ? defaultPath : app.getPath('downloads'),
      properties: ['openDirectory', 'createDirectory'],
    });
    return result.canceled || result.filePaths.length === 0 ? null : result.filePaths[0];
  });
  ipcMain.handle('shell:openPath', (_event, path: unknown) => (typeof path === 'string' ? shell.openPath(path) : '无效路径'));
  ipcMain.handle('shell:showInFolder', (_event, path: unknown) => {
    if (typeof path === 'string') {
      shell.showItemInFolder(path);
    }
  });
  ipcMain.handle('window:setTitleBarOverlay', (event, colors: unknown) => {
    if (process.platform !== 'win32') {
      return;
    }
    const window = BrowserWindow.fromWebContents(event.sender);
    const { color, symbolColor } = (colors ?? {}) as { color?: unknown; symbolColor?: unknown };
    const isHex = (value: unknown): value is string => typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value);
    if (window && !window.isDestroyed() && isHex(color) && isHex(symbolColor)) {
      window.setTitleBarOverlay({ color, symbolColor, height: TITLE_BAR_HEIGHT });
    }
  });
  ipcMain.handle('shell:openExternal', (_event, url: unknown) => {
    if (typeof url === 'string' && /^https:\/\//i.test(url)) {
      return shell.openExternal(url);
    }
    return undefined;
  });
}

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0 && host) {
    createWindow();
  }
});

app.on('window-all-closed', () => {
  // On macOS the app (and its downloads) keep running until it is quit.
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('before-quit', (event) => {
  if (quitting) {
    return;
  }
  event.preventDefault();
  quitting = true;
  for (const window of BrowserWindow.getAllWindows()) {
    saveWindowState(hostDir, window);
  }
  const stop = host ? host.stop() : Promise.resolve();
  stop
    .catch((error) => log(`engine stop failed: ${error instanceof Error ? error.message : error}`))
    .finally(() => app.quit());
});
