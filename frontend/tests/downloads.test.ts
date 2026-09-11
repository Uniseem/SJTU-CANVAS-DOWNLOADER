import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { useDownloadsStore } from '../src/stores/downloads'
import { usePreferencesStore } from '../src/stores/preferences'
import type { DownloadRequestItem } from '../src/types'
import { MemoryDirectory } from './fake-filesystem'

const { prepare, fetchMedia } = vi.hoisted(() => ({ prepare: vi.fn(), fetchMedia: vi.fn() }))
vi.mock('../src/services/api', () => ({
  api: { prepare }, isDemoMode: false, notifyApiUnauthorized: vi.fn(),
  ApiError: class ApiError extends Error {},
  getErrorMessage: (error: Error, fallback: string) => error?.message || fallback,
}))

function descriptor(item: DownloadRequestItem, index: number) {
  return { id: `ticket-${item.id}-${index}`, filename: item.type === 'video' ? 'upstream.mp4' : item.filename,
    directUrl: `https://media.test/${item.id}`, directSupported: true, proxyUrl: `/proxy/${item.id}`,
    source: item.type === 'video' ? `video:${item.track}` : 'file', size: 4, expiresAt: '2099-01-01', }
}
const file = (id: string, courseId = '42'): DownloadRequestItem => ({ id, type: 'file', filename: `${id}.pdf`, courseId, courseName: '测试课程' })
let picker: ReturnType<typeof vi.fn>
let store: ReturnType<typeof useDownloadsStore>
let storage: Map<string, string>

beforeEach(() => {
  setActivePinia(createPinia())
  storage = new Map()
  vi.stubGlobal('localStorage', { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) })
  picker = vi.fn().mockResolvedValue(new MemoryDirectory('默认目录').handle())
  vi.stubGlobal('window', { showDirectoryPicker: picker, setTimeout })
  vi.stubGlobal('fetch', fetchMedia)
  prepare.mockReset().mockImplementation(async (items: DownloadRequestItem[]) => ({ items: items.map(descriptor), failures: [] }))
  fetchMedia.mockReset().mockImplementation(async () => new Response(new Uint8Array([1, 2, 3, 4]), { headers: { 'Content-Length': '4' } }))
  store = useDownloadsStore()
})

afterEach(async () => {
  store.cancelPending()
  store.tasks.forEach((task) => store.cancel(task.taskId))
  await vi.waitFor(() => expect(store.pendingPrepareCount).toBe(0))
  vi.unstubAllGlobals()
})

describe('download destinations', () => {
  it('does not prepare or fetch anything when location selection is cancelled', async () => {
    picker.mockRejectedValue(new DOMException('cancel', 'AbortError'))
    expect(await store.enqueue([file('one')])).toBe(false)
    expect(prepare).not.toHaveBeenCalled()
    expect(fetchMedia).not.toHaveBeenCalled()
    expect(store.tasks).toHaveLength(0)
    expect(store.selectingDestination).toBe(false)
  })

  it('never silently downloads without directory API support', async () => {
    window.showDirectoryPicker = undefined
    expect(await store.enqueue([file('one')])).toBe(false)
    expect(store.prepareError).toContain('未选择保存位置')
    expect(prepare).not.toHaveBeenCalled()
  })

  it('prompts every batch and keeps queued tasks bound to their original directory', async () => {
    usePreferencesStore().concurrency = 1
    const a = new MemoryDirectory('A'), b = new MemoryDirectory('B')
    picker.mockResolvedValueOnce(a.handle()).mockResolvedValueOnce(b.handle())
    expect(await store.enqueue([file('one'), file('two')])).toBe(true)
    expect(await store.enqueue([file('three')])).toBe(true)
    await vi.waitFor(() => expect(store.tasks.filter((task) => task.status === 'completed')).toHaveLength(3))
    expect(picker).toHaveBeenCalledTimes(2)
    expect(a.files().map((entry) => entry.path)).toEqual(['测试课程 [42]/课程文件/one.pdf', '测试课程 [42]/课程文件/two.pdf'])
    expect(b.files().map((entry) => entry.path)).toEqual(['测试课程 [42]/课程文件/three.pdf'])
    expect([...a.files(), ...b.files()].every((entry) => entry.file.data.length === 4)).toBe(true)
    expect(prepare.mock.calls.every((call) => call[1] === 'auto')).toBe(true)
  })

  it('does not reuse a previous location after cancelling the next picker', async () => {
    await store.enqueue([file('one')])
    await vi.waitFor(() => expect(store.tasks[0]?.status).toBe('completed'))
    picker.mockRejectedValueOnce(new DOMException('cancel', 'AbortError'))
    const count = prepare.mock.calls.length
    expect(await store.enqueue([file('two')])).toBe(false)
    expect(prepare).toHaveBeenCalledTimes(count)
    expect(store.tasks).toHaveLength(1)
  })

  it('maps partial preparation failures to the correct course and view', async () => {
    const root = new MemoryDirectory('双视角')
    picker.mockResolvedValue(root.handle())
    const requests = [file('failed', '1'), { type: 'video' as const, id: 'v1', courseName: '视频课', courseId: '2', track: 'teacher' as const }]
    prepare.mockResolvedValueOnce({ items: [descriptor(requests[1], 0)], failures: [{ index: 0, message: '不可用' }] })
    await store.enqueue(requests)
    await vi.waitFor(() => expect(store.tasks[0]?.status).toBe('completed'))
    expect(root.files()[0].path).toMatch(/^视频课 \[2\]\/课堂录像\/.*\/教室摄像头.mp4$/)
    expect(store.tasks[0].request.id).toBe('v1')
    expect(store.prepareError).toContain('1 项未能加入')
  })

  it('uses the same chosen output when direct fetching falls back to proxy', async () => {
    const root = new MemoryDirectory('回退')
    picker.mockResolvedValue(root.handle())
    fetchMedia.mockRejectedValueOnce(new TypeError('CORS'))
    await store.enqueue([file('one')])
    await vi.waitFor(() => expect(store.tasks[0]?.status).toBe('completed'))
    expect(store.tasks[0].mode).toBe('proxy')
    expect(store.tasks[0].fallbackUsed).toBe(true)
    expect(root.files()).toHaveLength(1)
    expect(fetchMedia.mock.calls.map((call) => call[1].credentials)).toEqual(['omit', 'include'])
  })

  it('resumes a paused stream in its original file without another picker, including an immediate resume', async () => {
    const root = new MemoryDirectory('继续目录')
    picker.mockResolvedValue(root.handle())
    fetchMedia.mockImplementationOnce(async (_url: string, init: RequestInit) => new Response(new ReadableStream({
      start(controller) {
        controller.enqueue(new Uint8Array([1, 2]))
        init.signal!.addEventListener('abort', () => controller.error(new DOMException('paused', 'AbortError')), { once: true })
      },
    }), { headers: { 'Content-Length': '4' } }))
    fetchMedia.mockImplementationOnce(async () => new Response(new Uint8Array([3, 4]), {
      status: 206, headers: { 'Content-Length': '2', 'Content-Range': 'bytes 2-3/4' },
    }))
    await store.enqueue([file('one')])
    await vi.waitFor(() => expect(store.tasks[0]?.received).toBe(2))
    const id = store.tasks[0].taskId
    store.pause(id)
    store.resume(id)
    await vi.waitFor(() => expect(store.tasks[0]?.status).toBe('completed'))
    expect(picker).toHaveBeenCalledTimes(1)
    expect(prepare).toHaveBeenCalledTimes(1)
    expect(fetchMedia.mock.calls[1][1].headers.Range).toBe('bytes=2-')
    expect(root.files()).toHaveLength(1)
    expect(root.files()[0].file.data).toEqual(new Uint8Array([1, 2, 3, 4]))
  })

  it('asks for a new location and new ticket when retrying a failure', async () => {
    usePreferencesStore().fallbackToProxy = false
    fetchMedia.mockRejectedValueOnce(new TypeError('offline'))
    await store.enqueue([file('one')])
    await vi.waitFor(() => expect(store.tasks[0]?.status).toBe('failed'))
    const original = store.tasks[0].taskId
    const retryDirectory = new MemoryDirectory('重试目录')
    picker.mockResolvedValueOnce(retryDirectory.handle())
    await store.retry(original, 'proxy')
    await vi.waitFor(() => expect(store.tasks[0]?.status).toBe('completed'))
    expect(picker).toHaveBeenCalledTimes(2)
    expect(prepare).toHaveBeenCalledTimes(2)
    expect(store.tasks[0].destinationName).toBe('重试目录')
    expect(retryDirectory.files()).toHaveLength(1)
  })

  it('invalidates a pending picker when the queue/session is cancelled', async () => {
    let resolve!: (root: FileSystemDirectoryHandle) => void
    picker.mockReturnValueOnce(new Promise((done) => { resolve = done }))
    const pending = store.enqueue([file('one')])
    expect(store.selectingDestination).toBe(true)
    expect(await store.enqueue([file('two')])).toBe(false)
    store.cancelPending()
    resolve(new MemoryDirectory('late').handle())
    expect(await pending).toBe(false)
    expect(prepare).not.toHaveBeenCalled()
  })
})

describe('compact preference migration', () => {
  it('defaults to compact and reset restores it', () => {
    const preferences = usePreferencesStore()
    expect(preferences.compactCourseCards).toBe(true)
    preferences.compactCourseCards = false
    preferences.reset()
    expect(preferences.compactCourseCards).toBe(true)
  })

  it('migrates old density, keeps network choices, then honors new explicit density choices', async () => {
    storage.set('canvas-pocket-preferences-v1', JSON.stringify({ compactCourseCards: false, downloadMode: 'proxy', concurrency: 2 }))
    setActivePinia(createPinia())
    let preferences = usePreferencesStore()
    expect(preferences.compactCourseCards).toBe(true)
    expect(preferences.downloadMode).toBe('proxy')
    expect(preferences.concurrency).toBe(2)
    preferences.compactCourseCards = false
    await nextTick()
    setActivePinia(createPinia())
    preferences = usePreferencesStore()
    expect(preferences.compactCourseCards).toBe(false)
  })
})
