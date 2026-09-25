import { contextBridge, ipcRenderer, type IpcRendererEvent } from 'electron';

import type { CanvasBridge, EngineNotification, EngineState } from '@shared/protocol';

function subscribe<T>(channel: string, listener: (payload: T) => void): () => void {
  const wrapped = (_event: IpcRendererEvent, payload: T): void => listener(payload);
  ipcRenderer.on(channel, wrapped);
  return () => {
    ipcRenderer.removeListener(channel, wrapped);
  };
}

const bridge: CanvasBridge = {
  platform: process.platform,
  call: (method, params) => ipcRenderer.invoke('engine:call', method, params ?? {}),
  engineState: () => ipcRenderer.invoke('engine:state'),
  restartEngine: () => ipcRenderer.invoke('engine:restart'),
  onNotification: (listener) => subscribe<EngineNotification>('engine:notification', listener),
  onEngineState: (listener) => subscribe<EngineState>('engine:state', listener),
  appInfo: () => ipcRenderer.invoke('app:info'),
  chooseFolder: (defaultPath) => ipcRenderer.invoke('dialog:chooseFolder', defaultPath),
  openPath: (path) => ipcRenderer.invoke('shell:openPath', path),
  showInFolder: (path) => ipcRenderer.invoke('shell:showInFolder', path),
  openExternal: (url) => ipcRenderer.invoke('shell:openExternal', url),
};

contextBridge.exposeInMainWorld('canvas', bridge);
