import { demoApi } from '@/services/demo'
import type {
  Assignment,
  Course,
  CourseFile,
  Dashboard,
  DownloadDescriptor,
  DownloadRequestItem,
  Lesson,
  LessonSizes,
  VideoTrackKind,
  SessionView,
} from '@/types'

export const isDemoMode = import.meta.env.VITE_DEMO_MODE === 'true'
const unauthorizedHandlers = new Set<() => void>()
let authEpoch = 0

export function advanceApiAuthEpoch() { authEpoch += 1 }

export function onApiUnauthorized(handler: () => void) {
  unauthorizedHandlers.add(handler)
  return () => unauthorizedHandlers.delete(handler)
}

export function notifyApiUnauthorized() {
  advanceApiAuthEpoch()
  unauthorizedHandlers.forEach((handler) => handler())
}

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
    public readonly details?: unknown,
    public readonly retryAfterMs?: number,
  ) {
    super(message)
    this.name = 'ApiError'
  }
}

function parseRetryAfter(value: string | null) {
  if (!value) return undefined
  const seconds = Number(value)
  if (Number.isFinite(seconds) && seconds >= 0) return Math.max(1000, seconds * 1000)
  const timestamp = Date.parse(value)
  if (Number.isNaN(timestamp)) return undefined
  return Math.max(1000, timestamp - Date.now())
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const requestEpoch = authEpoch
  const response = await fetch(path, {
    credentials: 'include',
    headers: init?.body ? { 'Content-Type': 'application/json', ...init.headers } : init?.headers,
    ...init,
  })
  if (!response.ok) {
    if (response.status === 401 && requestEpoch === authEpoch) notifyApiUnauthorized()
    let details: unknown
    try {
      details = await response.json()
    } catch {
      details = await response.text().catch(() => undefined)
    }
    const nestedError = details && typeof details === 'object' && 'error' in details
      ? (details as { error?: unknown }).error
      : undefined
    const message =
      details && typeof details === 'object' && 'message' in details
        ? String((details as { message: unknown }).message)
        : nestedError && typeof nestedError === 'object' && 'message' in nestedError
          ? String((nestedError as { message: unknown }).message)
          : response.status === 401
            ? '登录状态已失效，请重新登录'
          : `请求失败（${response.status}）`
    throw new ApiError(message, response.status, details, parseRetryAfter(response.headers.get('Retry-After')))
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

function arrayFrom<T>(value: T[] | { items?: T[]; data?: T[] }, key?: string): T[] {
  if (Array.isArray(value)) return value
  if (key && key in value) {
    const keyed = (value as Record<string, unknown>)[key]
    if (Array.isArray(keyed)) return keyed as T[]
  }
  return value.items ?? value.data ?? []
}

export const api = {
  session: (signal?: AbortSignal) => (isDemoMode ? demoApi.session() : request<SessionView>('/api/session', { signal: signal ?? AbortSignal.timeout(15000) })),
  logout: () => (isDemoMode ? demoApi.logout() : request<void>('/api/session', { method: 'DELETE' })),
  dashboard: () => (isDemoMode ? demoApi.dashboard() : request<Dashboard>('/api/dashboard')),
  async courses(): Promise<Course[]> {
    if (isDemoMode) return demoApi.courses()
    const result = await request<Course[] | { courses?: Course[]; items?: Course[] }>('/api/courses')
    return arrayFrom(result, 'courses')
  },
  async files(courseId: string): Promise<CourseFile[]> {
    if (isDemoMode) return demoApi.files(courseId)
    const result = await request<CourseFile[] | { files?: CourseFile[]; items?: CourseFile[] }>(`/api/courses/${encodeURIComponent(courseId)}/files`)
    return arrayFrom(result, 'files')
  },
  async lessons(courseId: string): Promise<Lesson[]> {
    if (isDemoMode) return demoApi.lessons(courseId)
    const result = await request<Lesson[] | { lessons?: Lesson[]; items?: Lesson[] }>(`/api/courses/${encodeURIComponent(courseId)}/lessons`)
    return arrayFrom(result, 'lessons')
  },
  async lessonSizes(courseId: string, lessonId: string, tracks: VideoTrackKind[], signal: AbortSignal, refresh = false): Promise<LessonSizes> {
    if (isDemoMode) return { videoId: lessonId, tracks: Object.fromEntries(tracks.map((kind) => [kind, { status: 'ready', size: 256 * 1024 }])) }
    const query = new URLSearchParams({ tracks: tracks.join(','), refresh: String(refresh) })
    return request<LessonSizes>(`/api/courses/${encodeURIComponent(courseId)}/lessons/${encodeURIComponent(lessonId)}/sizes?${query}`, { signal })
  },
  async assignments(courseId: string): Promise<Assignment[]> {
    if (isDemoMode) return demoApi.assignments(courseId)
    const result = await request<Assignment[] | { assignments?: Assignment[]; items?: Assignment[] }>(`/api/courses/${encodeURIComponent(courseId)}/assignments`)
    return arrayFrom(result, 'assignments')
  },
  prepare: async (items: DownloadRequestItem[], preference: 'auto' | 'direct' | 'proxy' = 'auto'): Promise<{
    items: DownloadDescriptor[]
    failures?: Array<{ index: number; message: string }>
  }> => {
    if (isDemoMode) return demoApi.prepare(items)
    const payload = items.map((item) => item.type === 'video'
      ? { kind: 'video', courseId: item.courseId ?? '', lessonId: item.id, track: item.track ?? 'teacher' }
      : { kind: 'file', courseId: item.courseId ?? '', fileId: item.id })
    return request<{ items: DownloadDescriptor[]; failures?: Array<{ index: number; message: string }> }>('/api/downloads/prepare', {
      method: 'POST',
      body: JSON.stringify({ preference, items: payload }),
    })
  },
  startQr: (signal?: AbortSignal) => request<{ attemptId: string }>('/api/auth/qr', { method: 'POST', body: '{}', signal }),
  qrStatus: (attemptId: string, signal?: AbortSignal) => request<Record<string, unknown>>(`/api/auth/qr/${encodeURIComponent(attemptId)}`, { signal }),
  refreshQr: (attemptId: string, signal?: AbortSignal) => request<{ attemptId?: string } | void>(`/api/auth/qr/${encodeURIComponent(attemptId)}/refresh`, { method: 'POST', body: '{}', signal }),
}

export function getErrorMessage(error: unknown, fallback = '暂时无法完成操作，请稍后重试') {
  return error instanceof Error ? error.message : fallback
}
