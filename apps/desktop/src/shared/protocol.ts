/**
 * Wire types of the sjtu-canvas-engine JSON-RPC protocol (engine/src/rpc.rs,
 * engine/src/models.rs, engine/src/downloads) and of the bridge the preload
 * script exposes to the renderer as `window.canvas`.
 */

export const PROTOCOL_VERSION = 1;

export interface EngineErrorShape {
  /** invalid_params | unauthorized | forbidden | not_found | conflict |
   *  video_unavailable | upstream_error | upstream_unavailable | internal |
   *  user_error | method_not_found | parse_error, or from the host:
   *  engine_stopped | timeout | ipc */
  code: string;
  message: string;
  retry_after_seconds?: number;
}

export interface Profile {
  id: string;
  name: string;
  short_name?: string;
  avatar_url?: string;
}

export interface AccountInfo {
  authenticated: boolean;
  profile: Profile | null;
  /** False when no credential-store key protects the saved login; it then
   *  lasts only until the engine stops. */
  persisted: boolean;
  /** Set by account.get with verify: the login was confirmed with Canvas. */
  verified?: boolean;
}

export type LoginState =
  | 'preparing'
  | 'waiting'
  | 'reconnecting'
  | 'authorizing'
  | 'authorized'
  | 'expired'
  | 'cancelled'
  | 'error';

export interface LoginStatus {
  attempt_id: string;
  state: LoginState;
  message: string;
  generation: number;
  revision: number;
  /** The QR code as a base64 PNG while the state is `waiting`. */
  qr_png?: string;
  expires_at?: string;
}

export interface Course {
  id: string;
  name: string;
  course_code: string;
  start_at?: string;
  end_at?: string;
  term?: string;
  teacher?: string;
  /** active | invited_or_pending | completed */
  enrollment_state: string;
}

export interface Lesson {
  video_id: string;
  title: string;
  begin_time: string;
  end_time: string;
  classroom: string;
  audit_status: number;
  available: boolean;
}

export interface CourseFile {
  id: string;
  display_name: string;
  filename: string;
  size: number;
  content_type?: string;
  updated_at?: string;
}

export type Track = 'slides' | 'teacher' | 'composite';
export const TRACKS: Track[] = ['slides', 'teacher', 'composite'];

export interface TrackSize {
  status: 'ready' | 'missing' | 'unavailable';
  size: number | null;
}

export interface LessonSizes {
  video_id: string;
  tracks: Record<string, TrackSize>;
}

export type DownloadStatus = 'queued' | 'downloading' | 'paused' | 'completed' | 'failed' | 'cancelled';

export interface DownloadInfo {
  id: string;
  kind: 'video' | 'file';
  course_id: string;
  course_name: string;
  resource_id: string;
  track: Track | null;
  title: string;
  /** "第 01 讲 · 电脑屏幕" for videos, the file name for course files. */
  display_name: string;
  begin_time: string | null;
  destination: string;
  /** The final location once known; the file is `<path>.part` until done. */
  file_path: string | null;
  status: DownloadStatus;
  received: number;
  total: number | null;
  /** Bytes per second while downloading. */
  speed: number;
  error: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface DownloadProgress {
  id: string;
  received: number;
  total: number | null;
  speed: number;
}

export interface DownloadCounts {
  all: number;
  /** Not finished: queued, downloading or paused. */
  active: number;
  /** Queued or downloading. */
  running: number;
  completed: number;
  /** Failed or cancelled. */
  failed: number;
}

export interface DownloadList {
  items: DownloadInfo[];
  counts: DownloadCounts;
}

export type DownloadFilter = 'all' | 'active' | 'completed' | 'failed';

export type NewDownload =
  | {
      kind: 'video';
      course_id: string;
      course_name: string;
      lesson_id: string;
      title: string;
      begin_time?: string;
      track: Track;
      size?: number;
    }
  | {
      kind: 'file';
      course_id: string;
      course_name: string;
      file_id: string;
      title: string;
      size?: number;
    };

export interface SkippedItem {
  index: number;
  /** invalid | queued | downloaded */
  reason: string;
  message: string;
}

export interface CreateResult {
  created: DownloadInfo[];
  skipped: SkippedItem[];
}

export type ProxySettings = { mode: 'system' } | { mode: 'direct' } | { mode: 'custom'; url: string };

export interface Preferences {
  download_dir: string;
  ask_destination: boolean;
  concurrency: number;
  default_tracks: Track[];
  proxy: ProxySettings;
}

export interface SettingsInfo {
  preferences: Preferences;
  default_download_dir: string;
  concurrency_max: number;
  tracks: Track[];
  data_dir: string;
  engine_version: string;
  test_mode: boolean;
  fake_school: boolean;
}

export interface InitializeResult {
  protocol: number;
  version: string;
  data_dir: string;
  settings: SettingsInfo;
  account: AccountInfo;
}

/** A JSON-RPC notification from the engine (no id). */
export interface EngineNotification {
  method: string;
  params: unknown;
}

export type EngineStatus = 'starting' | 'running' | 'stopped';

/** The host's view of the engine process, pushed to the renderer. */
export interface EngineState {
  status: EngineStatus;
  /** Why the engine is not running. */
  error: string | null;
  version: string | null;
  dataDir: string;
  settings: SettingsInfo | null;
  account: AccountInfo | null;
}

export interface AppInfo {
  version: string;
  platform: 'win32' | 'darwin' | 'linux' | string;
  dataDir: string;
  logsDir: string;
  downloadsFolder: string;
}

export type CallResult = { ok: true; result: unknown } | { ok: false; error: EngineErrorShape };

/** What the preload script exposes as `window.canvas`. */
export interface CanvasBridge {
  platform: string;
  call(method: string, params?: unknown): Promise<CallResult>;
  engineState(): Promise<EngineState>;
  restartEngine(): Promise<EngineState>;
  onNotification(listener: (message: EngineNotification) => void): () => void;
  onEngineState(listener: (state: EngineState) => void): () => void;
  appInfo(): Promise<AppInfo>;
  chooseFolder(defaultPath?: string): Promise<string | null>;
  /** Opens a file with its default app; resolves to an error message or ''. */
  openPath(path: string): Promise<string>;
  showInFolder(path: string): Promise<void>;
  openExternal(url: string): Promise<void>;
}
