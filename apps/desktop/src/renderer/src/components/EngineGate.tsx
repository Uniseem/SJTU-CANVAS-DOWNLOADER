import { useState } from 'react';

import { Button, Spinner } from '@heroui/react';
import { FolderOpen, RotateCw } from 'lucide-react';

import { shell } from '../api';
import { useStore } from '../store';
import { ErrorNotice } from './common';

/** Shown while the engine starts and when it is not running. */
export function EngineGate() {
  const engineState = useStore((state) => state.engine);
  const restart = useStore((state) => state.restartEngine);
  const [busy, setBusy] = useState(false);

  if (!engineState || engineState.status === 'starting') {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-4 border-t border-border text-muted">
        <Spinner size="lg" color="accent" />
        <p className="text-sm">正在启动下载引擎…</p>
      </div>
    );
  }

  return (
    <div className="flex flex-1 items-center justify-center border-t border-border p-8">
      <div className="w-full max-w-lg space-y-4">
        <ErrorNotice title="下载引擎没有运行" message={engineState.error ?? '下载引擎已停止'} />
        <div className="flex gap-2">
          <Button
            variant="primary"
            isDisabled={busy}
            onPress={async () => {
              setBusy(true);
              try {
                await restart();
              } finally {
                setBusy(false);
              }
            }}
          >
            <RotateCw size={16} />
            重新启动
          </Button>
          <Button
            variant="secondary"
            onPress={async () => {
              const info = await shell.appInfo();
              await shell.openPath(info.logsDir);
            }}
          >
            <FolderOpen size={16} />
            打开日志文件夹
          </Button>
        </div>
      </div>
    </div>
  );
}
