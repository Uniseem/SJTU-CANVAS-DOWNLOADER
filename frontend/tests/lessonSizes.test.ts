import { afterEach, describe, expect, it, vi } from 'vitest'
import { createLessonSizeLoader, selectedSize } from '../src/utils/lessonSizes'
import type { LessonSizes, VideoTrackKind } from '../src/types'

const ready = (size: number) => ({ status: 'ready' as const, size })
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve() }
const cleanups: Array<() => void> = []
afterEach(() => { cleanups.splice(0).forEach((cleanup) => cleanup()); vi.restoreAllMocks() })

function fixture() {
  const pending: Array<{ id: string; tracks: VideoTrackKind[]; signal: AbortSignal; refresh?: boolean; resolve: (value: LessonSizes) => void; reject: (error: Error) => void }> = []
  const fetcher = vi.fn((_course: string, id: string, tracks: VideoTrackKind[], signal: AbortSignal, refresh?: boolean) => new Promise<LessonSizes>((resolve, reject) => pending.push({ id, tracks, signal, refresh, resolve, reject })))
  const loader = createLessonSizeLoader('87084', fetcher)
  cleanups.push(loader.dispose)
  const resolve = (index: number) => { const request = pending[index]; request.resolve({ videoId: request.id, tracks: Object.fromEntries(request.tracks.map((kind) => [kind, ready(kind === 'slides' ? 100 : 200)])) }) }
  return { loader, fetcher, pending, resolve }
}

describe('video size totals', () => {
  it('sums only selected views without double counting', () => {
    const tracks = { teacher: ready(300), slides: ready(200), composite: ready(400) }
    expect(selectedSize(tracks, ['slides', 'teacher']).size).toBe(500)
    expect(selectedSize(tracks, ['slides']).size).toBe(200)
    expect(selectedSize(tracks, ['teacher', 'teacher']).size).toBe(300)
  })
  it('never presents a partial or invalid total as a complete size', () => {
    expect(selectedSize({ teacher: ready(300) }, ['teacher', 'slides']).status).toBe('pending')
    expect(selectedSize({ teacher: ready(300), slides: { status: 'unavailable', size: null } }, ['teacher', 'slides']).size).toBeNull()
    expect(selectedSize({ slides: { status: 'missing', size: null } }, ['slides']).status).toBe('missing')
    expect(selectedSize({ teacher: ready(0) }, ['teacher']).status).toBe('unavailable')
    expect(selectedSize({ teacher: ready(Number.MAX_SAFE_INTEGER), slides: ready(1) }, ['teacher', 'slides']).status).toBe('unavailable')
  })
})

describe('visible video metadata queue', () => {
  it('loads only visible rows with at most two concurrent requests', async () => {
    const { loader, fetcher, resolve } = fixture()
    expect(fetcher).not.toHaveBeenCalled()
    for (const id of ['one', 'two', 'three']) loader.setVisible(id, true)
    expect(fetcher).toHaveBeenCalledTimes(2)
    resolve(0); await flush()
    expect(fetcher).toHaveBeenCalledTimes(3)
    expect(loader.summary('one', ['slides', 'teacher']).size).toBe(300)
    expect(fetcher.mock.calls[0][0]).toBe('87084')
  })
  it('does not start queued rows that scrolled out of view', async () => {
    const { loader, fetcher, resolve } = fixture()
    for (const id of ['one', 'two', 'three']) loader.setVisible(id, true)
    loader.setVisible('three', false)
    resolve(0); await flush()
    expect(fetcher).toHaveBeenCalledTimes(2)
  })
  it('reuses per-view sizes and fetches only newly selected views', async () => {
    const { loader, fetcher, resolve, pending } = fixture()
    loader.setVisible('one', true)
    loader.setTracks(['slides'])
    resolve(0); await flush()
    expect(loader.summary('one', ['slides']).size).toBe(100)
    loader.setTracks(['teacher', 'slides'])
    expect(fetcher).toHaveBeenCalledTimes(1)
    loader.setTracks(['composite'])
    expect(pending[1].tracks).toEqual(['composite'])
  })
  it('shows a retry on failure without retrying forever', async () => {
    const { loader, fetcher, pending } = fixture()
    loader.setVisible('one', true)
    pending[0].reject(new Error('network')); await flush()
    expect(loader.summary('one', ['teacher', 'slides']).text).toBe('重试大小')
    loader.setVisible('one', true)
    expect(fetcher).toHaveBeenCalledTimes(1)
    loader.retry('one')
    expect(fetcher).toHaveBeenCalledTimes(2)
    expect(pending[1].refresh).toBe(true)
  })
  it('does not accept another lesson response', async () => {
    const { loader, pending } = fixture()
    loader.setVisible('one', true)
    pending[0].resolve({ videoId: 'wrong', tracks: { teacher: ready(1), slides: ready(1) } }); await flush()
    expect(loader.summary('one', ['teacher', 'slides']).status).toBe('unavailable')
  })
  it('cancels background reads when leaving the video tab and resumes safely', async () => {
    const { loader, pending, resolve, fetcher } = fixture()
    loader.setVisible('one', true)
    loader.setEnabled(false)
    expect(pending[0].signal.aborted).toBe(true)
    resolve(0); await flush()
    expect(loader.rows.one.tracks.teacher).toBeUndefined()
    loader.setEnabled(true)
    expect(fetcher).toHaveBeenCalledTimes(2)
  })
  it('ignores responses after disposing the course page', async () => {
    const { loader, pending, resolve } = fixture()
    loader.setVisible('one', true)
    loader.dispose()
    expect(pending[0].signal.aborted).toBe(true)
    resolve(0); await flush()
    expect(loader.rows.one.tracks.teacher).toBeUndefined()
  })
})
