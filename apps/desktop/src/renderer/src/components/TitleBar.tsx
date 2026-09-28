import { cn } from '@heroui/react';
import { Download } from 'lucide-react';

import { isMac, isWindows } from '../api';

/**
 * The window's own title bar. On macOS the traffic lights sit at its left,
 * on Windows the native minimize/maximize/close buttons are drawn over its
 * right end (title bar overlay); the bar itself drags the window.
 */
export function TitleBar() {
  return (
    <header
      className={cn(
        'drag-region flex h-10 shrink-0 items-center gap-2 bg-surface-secondary text-sm select-none',
        isMac ? 'pl-[84px]' : 'pl-4',
        isWindows ? 'pr-36' : 'pr-4',
      )}
    >
      <div className="flex size-6 items-center justify-center rounded-md bg-accent text-accent-foreground">
        <Download size={13} />
      </div>
      <span className="font-semibold">Canvas 下载器</span>
      <span className="text-xs text-muted">上海交通大学</span>
    </header>
  );
}

/**
 * Tells the window the colors the title bar renders with, so the native
 * buttons Windows draws over it match the theme.
 */
export function reportTitleBarColors(): void {
  if (!isWindows) {
    return;
  }
  const probe = document.createElement('div');
  probe.className = 'bg-surface-secondary text-foreground';
  probe.style.position = 'absolute';
  probe.style.visibility = 'hidden';
  document.body.appendChild(probe);
  const style = getComputedStyle(probe);
  const color = toHex(style.backgroundColor);
  const symbolColor = toHex(style.color);
  probe.remove();
  if (color && symbolColor) {
    void window.canvas.setTitleBarOverlay({ color, symbolColor });
  }
}

/** Any CSS color as #rrggbb (a canvas normalizes opaque colors that way). */
function toHex(cssColor: string): string | null {
  const context = document.createElement('canvas').getContext('2d');
  if (!context) {
    return null;
  }
  context.fillStyle = '#000000';
  context.fillStyle = cssColor;
  const value = String(context.fillStyle);
  return /^#[0-9a-f]{6}$/i.test(value) ? value : null;
}
