export interface Profile {
  id: string
  name: string
  shortName?: string
  avatarUrl?: string | null
}

export interface SessionView {
  authenticated: boolean
  demo?: boolean
  profile?: Profile | null
}

export interface Course {
  id: string
  name: string
  courseCode?: string
  startAt?: string | null
  endAt?: string | null
  enrollmentState?: string
  term?: string
  teacher?: string
  color?: string
  fileCount?: number
  lessonCount?: number
}

export interface TodoItem {
  id: string
  title: string
  courseName: string
  dueAt?: string | null
  pointsPossible?: number | null
  submitted: boolean
}

export interface Dashboard {
  profile: Profile
  courses: Course[]
  todos: TodoItem[]
}

export interface CourseFile {
  id: string
  displayName: string
  filename: string
  size: number
  contentType?: string | null
  updatedAt?: string | null
}

export interface Lesson {
  videoId: string
  title: string
  beginTime: string
  endTime: string
  classroom: string
  auditStatus: number
  available: boolean
  source?: 'resource' | 'canvas-lti' | 'historical'
  size?: number
}

export type VideoTrackKind = 'teacher' | 'slides' | 'composite'
export interface VideoTrackSize { status: 'ready' | 'missing' | 'unavailable'; size: number | null }
export interface LessonSizes { videoId: string; tracks: Partial<Record<VideoTrackKind, VideoTrackSize>> }

export interface Assignment {
  id: string
  name: string
  dueAt?: string | null
  pointsPossible?: number | null
  submissionState: string
}

export type DownloadSource = 'file' | 'video'

export interface DownloadRequestItem {
  id: string
  type: DownloadSource
  track?: 'teacher' | 'slides' | 'composite'
  filename?: string
  courseId?: string
  title?: string
  courseName?: string
  beginTime?: string
}

export interface DownloadDescriptor {
  id: string
  filename: string
  size?: number | null
  directUrl?: string | null
  proxyUrl: string
  expiresAt: string
  source: DownloadSource | string
  directSupported: boolean
}

export type DownloadMode = 'direct' | 'proxy'
export type DownloadStatus = 'queued' | 'preparing' | 'downloading' | 'paused' | 'completed' | 'failed' | 'cancelled'

export interface DownloadTask extends DownloadDescriptor {
  taskId: string
  status: DownloadStatus
  mode: DownloadMode
  received: number
  total: number | null
  speed: number
  error?: string
  createdAt: number
  completedAt?: number
  fallbackUsed?: boolean
  etag?: string
  relativePath: string
  destinationName: string
  request: DownloadRequestItem
}

export type LoginStage = 'idle' | 'starting' | 'qr' | 'scanned' | 'confirming' | 'success' | 'expired' | 'error'

export interface LoginState {
  attemptId: string | null
  stage: LoginStage
  qrContent: string | null
  message: string
  expiresAt?: string | null
}
