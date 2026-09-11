import type {
  Assignment,
  Course,
  CourseFile,
  Dashboard,
  DownloadDescriptor,
  DownloadRequestItem,
  Lesson,
  SessionView,
} from '@/types'

const sleep = (ms = 260) => new Promise((resolve) => window.setTimeout(resolve, ms))

export const demoCourses: Course[] = [
  {
    id: 'cs101',
    name: '数据结构与算法',
    courseCode: 'CS2308-01',
    term: '2026 秋季学期',
    teacher: '陈老师',
    enrollmentState: 'active',
    lessonCount: 14,
    fileCount: 26,
    color: '#214ee5',
  },
  {
    id: 'math201',
    name: '概率论与数理统计',
    courseCode: 'MA212-02',
    term: '2026 秋季学期',
    teacher: '何老师',
    enrollmentState: 'active',
    lessonCount: 12,
    fileCount: 19,
    color: '#168568',
  },
  {
    id: 'ai301',
    name: '人工智能基础',
    courseCode: 'AI3601-01',
    term: '2026 秋季学期',
    teacher: '周老师',
    enrollmentState: 'active',
    lessonCount: 16,
    fileCount: 31,
    color: '#c56b16',
  },
  {
    id: 'eng102',
    name: '学术英语写作',
    courseCode: 'FL1204-07',
    term: '2026 秋季学期',
    teacher: '林老师',
    enrollmentState: 'active',
    lessonCount: 8,
    fileCount: 12,
    color: '#7b50c7',
  },
  {
    id: 'phy110',
    name: '大学物理（荣誉）',
    courseCode: 'PH001-03',
    term: '2026 秋季学期',
    teacher: '吴老师',
    enrollmentState: 'active',
    lessonCount: 13,
    fileCount: 22,
    color: '#3177a9',
  },
  {
    id: 'hist101',
    name: '中国近现代史纲要',
    courseCode: 'TH020-14',
    term: '2026 秋季学期',
    teacher: '杨老师',
    enrollmentState: 'active',
    lessonCount: 9,
    fileCount: 15,
    color: '#c94545',
  },
]

const files: CourseFile[] = [
  { id: 'f1', displayName: '第一章 · 算法分析', filename: '01-算法分析.pdf', size: 8_936_112, contentType: 'application/pdf', updatedAt: '2026-08-18T08:32:00Z' },
  { id: 'f2', displayName: '第二章 · 线性表', filename: '02-线性表与链表.pdf', size: 13_204_889, contentType: 'application/pdf', updatedAt: '2026-08-17T03:10:00Z' },
  { id: 'f3', displayName: '课程代码模板', filename: 'starter-code.zip', size: 2_619_340, contentType: 'application/zip', updatedAt: '2026-08-16T12:20:00Z' },
  { id: 'f4', displayName: '第一次习题答案', filename: 'week-01-solution.pdf', size: 4_406_222, contentType: 'application/pdf', updatedAt: '2026-08-15T02:45:00Z' },
  { id: 'f5', displayName: '课堂补充：复杂度证明', filename: 'complexity-notes.pdf', size: 6_117_940, contentType: 'application/pdf', updatedAt: '2026-08-12T09:05:00Z' },
]

const lessons: Lesson[] = [
  { videoId: 'v1', title: '01 · 课程导论与复杂度', beginTime: '2026-08-18T00:00:00Z', endTime: '2026-08-18T01:40:00Z', classroom: '东上院 100', auditStatus: 1, available: true, size: 842_000_000 },
  { videoId: 'v2', title: '02 · 抽象数据类型', beginTime: '2026-08-16T00:00:00Z', endTime: '2026-08-16T01:40:00Z', classroom: '东上院 100', auditStatus: 1, available: true, size: 781_000_000 },
  { videoId: 'v3', title: '03 · 数组、链表与游标', beginTime: '2026-08-14T00:00:00Z', endTime: '2026-08-14T01:40:00Z', classroom: '东上院 100', auditStatus: 1, available: true, size: 914_000_000 },
  { videoId: 'v4', title: '04 · 栈与队列', beginTime: '2026-08-11T00:00:00Z', endTime: '2026-08-11T01:40:00Z', classroom: '东上院 100', auditStatus: 0, available: false },
]

const assignments: Assignment[] = [
  { id: 'a1', name: '作业 01 · 渐进分析', dueAt: '2026-08-23T15:59:00Z', pointsPossible: 100, submissionState: 'unsubmitted' },
  { id: 'a2', name: '编程练习 · 链表实验', dueAt: '2026-08-28T15:59:00Z', pointsPossible: 100, submissionState: 'unsubmitted' },
  { id: 'a3', name: '随堂测验 · 复杂度', dueAt: '2026-08-15T15:59:00Z', pointsPossible: 20, submissionState: 'submitted' },
]

let demoAuthenticated = true

export const demoApi = {
  async session(): Promise<SessionView> {
    await sleep(180)
    return {
      authenticated: demoAuthenticated,
      demo: true,
      profile: demoAuthenticated ? { id: '20260001', name: '徐同学', shortName: '徐同学' } : null,
    }
  },
  async logout(): Promise<void> {
    await sleep(160)
    demoAuthenticated = false
  },
  async login(): Promise<void> {
    await sleep(180)
    demoAuthenticated = true
  },
  async dashboard(): Promise<Dashboard> {
    await sleep()
    return {
      profile: { id: '20260001', name: '徐同学', shortName: '徐同学' },
      courses: demoCourses,
      todos: [
        { id: 't1', title: '作业 01 · 渐进分析', courseName: '数据结构与算法', dueAt: '2026-08-23T15:59:00Z', pointsPossible: 100, submitted: false },
        { id: 't2', title: '阅读报告 · 语言模型', courseName: '人工智能基础', dueAt: '2026-08-25T15:59:00Z', pointsPossible: 20, submitted: false },
        { id: 't3', title: 'Problem Set 2', courseName: '概率论与数理统计', dueAt: '2026-08-27T15:59:00Z', pointsPossible: 100, submitted: false },
      ],
    }
  },
  async courses(): Promise<Course[]> {
    await sleep()
    return demoCourses
  },
  async files(_courseId: string): Promise<CourseFile[]> {
    await sleep()
    return files
  },
  async lessons(_courseId: string): Promise<Lesson[]> {
    await sleep()
    return lessons
  },
  async assignments(_courseId: string): Promise<Assignment[]> {
    await sleep()
    return assignments
  },
  async prepare(items: DownloadRequestItem[]): Promise<{ items: DownloadDescriptor[] }> {
    await sleep(420)
    return {
      items: items.map((item, index) => ({
        id: item.id,
        filename: item.filename ?? `${item.title ?? item.id}${item.track ? `-${item.track}` : ''}.${item.type === 'video' ? 'mp4' : 'pdf'}`,
        size: item.type === 'video' ? 120_000_000 + index * 24_000_000 : 7_000_000 + index * 800_000,
        directUrl: `/demo-assets/${encodeURIComponent(item.id)}`,
        proxyUrl: `/demo-assets/${encodeURIComponent(item.id)}?proxy=1`,
        expiresAt: new Date(Date.now() + 15 * 60_000).toISOString(),
        source: item.type === 'video' ? `video:${item.track ?? 'teacher'}` : item.type,
        directSupported: true,
      })),
    }
  },
}
