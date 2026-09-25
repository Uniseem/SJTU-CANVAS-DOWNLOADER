import { spawn, type ChildProcess } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { createInterface } from 'node:readline';

import type { EngineErrorShape, EngineNotification } from '@shared/protocol';

/** A request the engine answered with an error, or could not be sent. */
export class RpcFailure extends Error {
  constructor(readonly error: EngineErrorShape) {
    super(error.message);
    this.name = 'RpcFailure';
  }
}

export interface EngineExit {
  code: number | null;
  signal: NodeJS.Signals | null;
  error: string | null;
}

interface Pending {
  resolve(value: unknown): void;
  reject(error: unknown): void;
  timer: NodeJS.Timeout;
}

const delay = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * One engine process, driven over newline-delimited JSON-RPC on its
 * stdin/stdout. Notifications (messages without an id) are emitted as
 * `notification`; the end of the process as `exit`.
 */
export class EngineProcess extends EventEmitter {
  private child: ChildProcess | null = null;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private exited = false;
  private stderrTail = '';

  constructor(
    private readonly executable: string,
    private readonly dataDir: string,
    private readonly log: (line: string) => void,
  ) {
    super();
  }

  start(): void {
    const child = spawn(this.executable, ['--data-dir', this.dataDir], {
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
      env: process.env,
    });
    this.child = child;
    const lines = createInterface({ input: child.stdout!, crlfDelay: Infinity });
    lines.on('line', (line) => this.receive(line));
    child.stderr!.setEncoding('utf8');
    child.stderr!.on('data', (chunk: string) => {
      this.stderrTail = (this.stderrTail + chunk).slice(-4000);
      for (const line of chunk.split(/\r?\n/)) {
        if (line.trim()) {
          this.log(`[engine] ${line}`);
        }
      }
    });
    child.on('error', (error) => {
      this.log(`engine process error: ${error.message}`);
      this.finish({ code: null, signal: null, error: error.message });
    });
    child.on('exit', (code, signal) => this.finish({ code, signal, error: null }));
  }

  get running(): boolean {
    return this.child !== null && !this.exited;
  }

  /** The last line the engine wrote to stderr, e.g. a start-up failure. */
  get lastStderrLine(): string {
    const lines = this.stderrTail
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
    return lines.at(-1) ?? '';
  }

  call<T>(method: string, params: unknown = {}, timeoutMs = 120_000): Promise<T> {
    const child = this.child;
    if (!child || !this.running || !child.stdin) {
      return Promise.reject(new RpcFailure({ code: 'engine_stopped', message: '下载引擎未运行' }));
    }
    const id = this.nextId;
    this.nextId += 1;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new RpcFailure({ code: 'timeout', message: `请求 ${method} 超时` }));
      }, timeoutMs);
      this.pending.set(id, { resolve: (value) => resolve(value as T), reject, timer });
      const line = JSON.stringify({ id, method, params: params ?? {} });
      child.stdin!.write(`${line}\n`, (error) => {
        if (error && this.pending.delete(id)) {
          clearTimeout(timer);
          reject(new RpcFailure({ code: 'engine_stopped', message: `无法向下载引擎发送请求：${error.message}` }));
        }
      });
    });
  }

  /** Asks the engine to stop and waits for it; kills it after five seconds. */
  async stop(): Promise<void> {
    const child = this.child;
    if (!child || this.exited) {
      return;
    }
    const exited = new Promise<void>((resolve) => {
      if (this.exited) {
        resolve();
      } else {
        this.once('exit', () => resolve());
      }
    });
    try {
      await Promise.race([this.call('engine.shutdown', {}, 3_000), delay(3_000)]);
    } catch {
      // The engine also stops when stdin closes.
    }
    try {
      child.stdin?.end();
    } catch {
      // Already closed.
    }
    const stopped = await Promise.race([exited.then(() => true), delay(5_000).then(() => false)]);
    if (!stopped) {
      this.log('engine did not stop in time; killing it');
      child.kill();
      await Promise.race([exited, delay(2_000)]);
    }
  }

  private receive(line: string): void {
    let message: { id?: unknown; result?: unknown; error?: EngineErrorShape; method?: unknown; params?: unknown };
    try {
      message = JSON.parse(line);
    } catch {
      this.log(`engine wrote a non-JSON line: ${line.slice(0, 200)}`);
      return;
    }
    if (typeof message.id === 'number') {
      const waiting = this.pending.get(message.id);
      if (!waiting) {
        return;
      }
      this.pending.delete(message.id);
      clearTimeout(waiting.timer);
      if (message.error) {
        waiting.reject(new RpcFailure(message.error));
      } else {
        waiting.resolve(message.result);
      }
      return;
    }
    if (typeof message.method === 'string') {
      const notification: EngineNotification = { method: message.method, params: message.params };
      this.emit('notification', notification);
    }
  }

  private finish(exit: EngineExit): void {
    if (this.exited) {
      return;
    }
    this.exited = true;
    for (const waiting of this.pending.values()) {
      clearTimeout(waiting.timer);
      waiting.reject(new RpcFailure({ code: 'engine_stopped', message: '下载引擎已退出' }));
    }
    this.pending.clear();
    this.emit('exit', exit);
  }
}
