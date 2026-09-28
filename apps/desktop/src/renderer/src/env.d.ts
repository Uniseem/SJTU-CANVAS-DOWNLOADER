/// <reference types="vite/client" />

import type { CanvasBridge } from '@shared/protocol';

declare global {
  interface Window {
    canvas: CanvasBridge;
  }
}

export {};
