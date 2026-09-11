import { describe, expect, it } from 'vitest'
import { allocateDownloadFile, DEFAULT_VIDEO_TRACKS, downloadLayout, safeSegment } from '../src/utils/downloadLayout'
import { MemoryDirectory } from './fake-filesystem'

describe('download organization', () => {
  it('defaults to screen and classroom, with one lecture directory and two media files', () => {
    expect(DEFAULT_VIDEO_TRACKS).toEqual(['slides', 'teacher'])
    const layouts = DEFAULT_VIDEO_TRACKS.map((track) => downloadLayout({
      type: 'video', id: 'opaque_id', courseId: '87084', courseName: '常微分方程',
      title: '第74讲', beginTime: '2026-06-18T10:55:00Z', track,
    }, 'upstream.mp4'))
    expect(layouts[0].directories).toEqual(layouts[1].directories)
    expect(layouts[0].directories[0]).toBe('常微分方程 [87084]')
    expect(layouts[0].directories[2]).toMatch(/^2026-06-18_18-55 第74讲/)
    expect(layouts.map((layout) => layout.filename)).toEqual(['电脑屏幕.mp4', '教室摄像头.mp4'])
  })

  it('keeps course files separate and preserves extension on long names', () => {
    const result = downloadLayout({ type: 'file', id: 'f1', courseName: '数学', courseId: '42' }, '讲'.repeat(250) + '.pdf')
    expect(result.directories).toEqual(['数学 [42]', '课程文件'])
    expect(result.filename).toHaveLength(114)
    expect(result.filename.endsWith('.pdf')).toBe(true)
  })

  it('sanitizes traversal, reserved device names, and trailing dots after truncation', () => {
    for (const input of ['../x\\y', '..', 'a:b?c', 'NUL', 'con.pdf', 'COM1', 'LPT¹.txt', 'a'.repeat(69) + '.z']) {
      const value = safeSegment(input)
      expect(value).not.toMatch(/[<>:"/\\|?*\u0000-\u001f]/)
      expect(value).not.toMatch(/[. ]$/)
      expect(value).not.toMatch(/^(nul|con|com1|lpt¹)(\.|$)/i)
      expect(value).not.toBe('..')
    }
  })

  it('distinguishes same-titled lectures by stable resource id', () => {
    const item = { type: 'video' as const, courseId: '42', title: '同名讲次' }
    expect(downloadLayout({ ...item, id: 'one' }, 'a.mp4').directories)
      .not.toEqual(downloadLayout({ ...item, id: 'two' }, 'a.mp4').directories)
  })

  it('preserves existing files and allocates distinct names concurrently', async () => {
    const root = new MemoryDirectory('root')
    const directory = await root.getDirectoryHandle('course', { create: true })
    const existing = await directory.getFileHandle('notes.pdf', { create: true })
    existing.data = new Uint8Array([42])
    const allocated = await Promise.all([1, 2].map(() => allocateDownloadFile(root.handle(), ['course'], 'notes.pdf')))
    expect(allocated.map((entry) => entry.relativePath)).toEqual(['course/notes (2).pdf', 'course/notes (3).pdf'])
    expect(existing.data).toEqual(new Uint8Array([42]))
  })

  it('treats a colliding directory as occupied, but does not ignore permissions errors', async () => {
    const root = new MemoryDirectory('root')
    await root.getDirectoryHandle('notes.pdf', { create: true })
    expect((await allocateDownloadFile(root.handle(), [], 'notes.pdf')).filename).toBe('notes (2).pdf')
    root.getFileHandle = async () => { throw new DOMException('No permission', 'NotAllowedError') }
    await expect(allocateDownloadFile(root.handle(), [], 'other.pdf')).rejects.toMatchObject({ name: 'NotAllowedError' })
  })
})
