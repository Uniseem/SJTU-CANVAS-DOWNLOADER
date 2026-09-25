import { EventEmitter } from 'node:events';

import { app } from 'electron';

import type { AccountInfo, EngineNotification, EngineState, InitializeResult } from '@shared/protocol';

import { EngineProcess, RpcFailure, type EngineExit } from './engine';
import { locateEngine } from './paths';

/**
 * Supervises the engine process: starts it, runs `engine.initialize`, reports
 * its state to the windows and restarts it on request. Notifications from
 * the engine are re-emitted as `notification`, state changes as `state`.
 */
export class EngineHost extends EventEmitter {
  state: EngineState;
  private engine: EngineProcess | null = null;
  private starting: Promise<EngineState> | null = null;
  private stopping = false;

  constructor(
    private readonly dataDir: string,
    private readonly sessionKey: string | null,
    private readonly log: (line: string) => void,
  ) {
    super();
    this.state = { status: 'starting', error: null, version: null, dataDir, settings: null, account: null };
  }

  start(): Promise<EngineState> {
    if (this.engine?.running) {
      return Promise.resolve(this.state);
    }
    if (!this.starting) {
      this.starting = this.launch().finally(() => {
        this.starting = null;
      });
    }
    return this.starting;
  }

  private async launch(): Promise<EngineState> {
    this.setState({ status: 'starting', error: null });
    let executable: string;
    try {
      executable = locateEngine();
    } catch (error) {
      this.setState({ status: 'stopped', error: describe(error) });
      return this.state;
    }
    this.log(`starting engine ${executable} with data in ${this.dataDir}`);
    const engine = new EngineProcess(executable, this.dataDir, this.log);
    engine.on('notification', (notification: EngineNotification) => {
      if (notification.method === 'account.changed') {
        this.setState({ account: notification.params as AccountInfo });
      }
      this.emit('notification', notification);
    });
    engine.on('exit', (exit: EngineExit) => {
      if (this.engine !== engine) {
        return;
      }
      this.engine = null;
      if (this.stopping) {
        return;
      }
      const reason = exit.error ?? (exit.signal ? `信号 ${exit.signal}` : `退出码 ${exit.code}`);
      const detail = engine.lastStderrLine;
      this.log(`engine exited unexpectedly (${reason})`);
      this.setState({ status: 'stopped', error: `下载引擎意外退出（${reason}）${detail ? `：${detail}` : ''}` });
    });
    this.engine = engine;
    engine.start();
    try {
      const result = await engine.call<InitializeResult>(
        'engine.initialize',
        { session_key: this.sessionKey, downloads_folder: app.getPath('downloads') },
        30_000,
      );
      if (result.protocol !== 1) {
        throw new Error(`引擎协议版本 ${result.protocol} 与应用不匹配，请重新安装`);
      }
      this.log(`engine ${result.version} ready`);
      this.setState({
        status: 'running',
        error: null,
        version: result.version,
        settings: result.settings,
        account: result.account,
      });
    } catch (error) {
      const detail = engine.lastStderrLine;
      const message = describe(error);
      this.log(`engine initialization failed: ${message}`);
      this.stopping = true;
      await engine.stop().catch(() => undefined);
      this.stopping = false;
      if (this.engine === engine) {
        this.engine = null;
      }
      this.setState({
        status: 'stopped',
        error: `无法启动下载引擎：${detail && message.includes('已退出') ? detail : message}`,
      });
    }
    return this.state;
  }

  call<T>(method: string, params: unknown): Promise<T> {
    const engine = this.engine;
    if (!engine?.running) {
      return Promise.reject(
        new RpcFailure({ code: 'engine_stopped', message: this.state.error ?? '下载引擎未运行，请在设置中重新启动' }),
      );
    }
    return engine.call<T>(method, params);
  }

  async stop(): Promise<void> {
    this.stopping = true;
    try {
      const engine = this.engine;
      this.engine = null;
      if (engine) {
        await engine.stop();
      }
      this.setState({ status: 'stopped', error: null });
    } finally {
      this.stopping = false;
    }
  }

  async restart(): Promise<EngineState> {
    await this.stop();
    return this.start();
  }

  private setState(patch: Partial<EngineState>): void {
    this.state = { ...this.state, ...patch };
    this.emit('state', this.state);
  }
}

export function describe(error: unknown): string {
  if (error instanceof RpcFailure) {
    return error.error.message;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

export function toErrorShape(error: unknown): { code: string; message: string; retry_after_seconds?: number } {
  if (error instanceof RpcFailure) {
    return error.error;
  }
  return { code: 'ipc', message: describe(error) };
}
