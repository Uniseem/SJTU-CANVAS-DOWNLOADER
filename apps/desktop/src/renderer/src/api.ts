import type {
  AccountInfo,
  Course,
  CourseFile,
  CreateResult,
  DownloadFilter,
  DownloadInfo,
  DownloadList,
  EngineErrorShape,
  Lesson,
  LessonSizes,
  LoginStatus,
  NewDownload,
  Preferences,
  SettingsInfo,
  Track,
} from '@shared/protocol';

/** An error answered by the engine (or by the host on its behalf). */
export class EngineError extends Error {
  readonly code: string;
  readonly retryAfter: number | undefined;

  constructor(shape: EngineErrorShape) {
    super(shape.message);
    this.name = 'EngineError';
    this.code = shape.code;
    this.retryAfter = shape.retry_after_seconds;
  }
}

export async function call<T>(method: string, params?: unknown): Promise<T> {
  const result = await window.canvas.call(method, params);
  if (result.ok) {
    return result.result as T;
  }
  throw new EngineError(result.error);
}

export function errorMessage(error: unknown): string {
  if (error instanceof EngineError) {
    return error.retryAfter ? `${error.message}（约 ${error.retryAfter} 秒后可重试）` : error.message;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

export function isUnauthorized(error: unknown): boolean {
  return error instanceof EngineError && error.code === 'unauthorized';
}

export const engine = {
  courses: () => call<{ courses: Course[] }>('courses.list').then((result) => result.courses),
  lessons: (courseId: string) =>
    call<{ lessons: Lesson[] }>('courses.lessons', { course_id: courseId }).then((result) => result.lessons),
  files: (courseId: string) =>
    call<{ files: CourseFile[] }>('courses.files', { course_id: courseId }).then((result) => result.files),
  sizes: (courseId: string, lessonId: string, tracks: Track[], refresh = false) =>
    call<LessonSizes>('lessons.sizes', { course_id: courseId, lesson_id: lessonId, tracks, refresh }),
  createDownloads: (items: NewDownload[], destination?: string) =>
    call<CreateResult>('downloads.create', destination ? { items, destination } : { items }),
  listDownloads: (filter: DownloadFilter = 'all', query?: string) =>
    call<DownloadList>('downloads.list', query ? { filter, query } : { filter }),
  pause: (id: string) => call<DownloadInfo>('downloads.pause', { id }),
  resume: (id: string) => call<DownloadInfo>('downloads.resume', { id }),
  cancel: (id: string) => call<DownloadInfo>('downloads.cancel', { id }),
  retry: (id: string) => call<DownloadInfo>('downloads.retry', { id }),
  remove: (id: string) => call<{ removed: boolean }>('downloads.remove', { id }),
  pauseAll: () => call<{ count: number }>('downloads.pauseAll'),
  resumeAll: () => call<{ count: number }>('downloads.resumeAll'),
  clearCompleted: () => call<{ count: number }>('downloads.clearCompleted'),
  settings: () => call<SettingsInfo>('settings.get'),
  updateSettings: (preferences: Preferences) => call<SettingsInfo>('settings.update', { preferences }),
  account: (verify = false) => call<AccountInfo>('account.get', { verify }),
  logout: () => call<AccountInfo>('account.logout'),
  loginStart: () => call<LoginStatus>('login.start'),
  loginRefresh: () => call<Record<string, never>>('login.refresh'),
  loginCancel: () => call<Record<string, never>>('login.cancel'),
  loginStatus: () => call<LoginStatus | null>('login.status'),
};

export const shell = {
  chooseFolder: (defaultPath?: string) => window.canvas.chooseFolder(defaultPath),
  openPath: (path: string) => window.canvas.openPath(path),
  showInFolder: (path: string) => window.canvas.showInFolder(path),
  openExternal: (url: string) => window.canvas.openExternal(url),
  appInfo: () => window.canvas.appInfo(),
};

export const platform = window.canvas.platform;
export const isMac = platform === 'darwin';
