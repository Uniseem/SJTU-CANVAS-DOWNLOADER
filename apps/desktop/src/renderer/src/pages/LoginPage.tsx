import { useEffect, useRef, useState } from 'react';

import { Button, Chip, Spinner } from '@heroui/react';
import { RotateCw, X } from 'lucide-react';

import { engine, errorMessage } from '../api';
import { useStore } from '../store';

const ACTIVE_STATES = new Set(['preparing', 'waiting', 'reconnecting', 'authorizing', 'authorized']);

export function LoginPage() {
  const login = useStore((state) => state.login);
  const fakeSchool = useStore((state) => state.settings?.fake_school ?? false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const started = useRef(false);

  const start = async (): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const status = await engine.loginStart();
      useStore.setState({ login: status });
    } catch (failure) {
      setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (started.current) {
      return;
    }
    started.current = true;
    void (async () => {
      try {
        const current = await engine.loginStatus();
        if (current && ACTIVE_STATES.has(current.state)) {
          useStore.setState({ login: current });
          return;
        }
      } catch {
        // Fall through to a fresh attempt.
      }
      await start();
    })();
  }, []);

  const state = login?.state ?? 'preparing';
  const showQr = state === 'waiting' && login?.qr_png;
  const pending = state === 'preparing' || state === 'reconnecting' || state === 'authorizing' || state === 'authorized';
  const ended = state === 'expired' || state === 'cancelled' || state === 'error';

  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-6 border-t border-border px-8">
      <div className="flex w-full max-w-sm flex-col items-center gap-5 text-center">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">登录 Canvas</h1>
          <p className="mt-2 text-sm text-muted">打开「交我办」App，扫描下面的二维码并在手机上确认。</p>
        </div>

        <div className="relative flex size-64 items-center justify-center rounded-3xl border border-border bg-white p-3 shadow-surface">
          {showQr ? (
            <img
              src={`data:image/png;base64,${login.qr_png}`}
              alt="交我办登录二维码"
              className="size-full rounded-xl"
              draggable={false}
            />
          ) : pending || busy ? (
            <div className="flex flex-col items-center gap-3 text-muted">
              <Spinner size="lg" color="accent" />
              <span className="text-xs">{state === 'authorizing' || state === 'authorized' ? '正在验证身份…' : '正在获取二维码…'}</span>
            </div>
          ) : (
            <div className="flex flex-col items-center gap-3 px-4 text-muted">
              <span className="text-sm">二维码已失效</span>
              <Button size="sm" variant="primary" onPress={start} isDisabled={busy}>
                重新获取
              </Button>
            </div>
          )}
        </div>

        <div className="min-h-6">
          {error ? (
            <Chip color="danger" variant="soft">
              {error}
            </Chip>
          ) : login ? (
            <Chip color={ended ? 'warning' : state === 'authorized' ? 'success' : 'default'} variant="soft">
              {login.message}
            </Chip>
          ) : null}
        </div>

        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            isDisabled={busy || state === 'authorizing' || state === 'authorized'}
            onPress={async () => {
              setBusy(true);
              setError(null);
              try {
                await engine.loginRefresh();
              } catch (failure) {
                setError(errorMessage(failure));
              } finally {
                setBusy(false);
              }
            }}
          >
            <RotateCw size={14} />
            刷新二维码
          </Button>
          {state === 'waiting' || state === 'preparing' || state === 'reconnecting' ? (
            <Button
              variant="ghost"
              size="sm"
              onPress={async () => {
                try {
                  await engine.loginCancel();
                } catch (failure) {
                  setError(errorMessage(failure));
                }
              }}
            >
              <X size={14} />
              取消
            </Button>
          ) : null}
        </div>

        <p className="max-w-xs text-xs leading-relaxed text-muted">
          应用只保存 Canvas 的登录状态（加密存放在本机），不读取、不保存 jAccount 的账号与密码。
        </p>
        {fakeSchool ? (
          <Chip color="warning" variant="soft" size="sm">
            演示模式：使用虚拟的学校数据
          </Chip>
        ) : null}
      </div>
    </div>
  );
}
