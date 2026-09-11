import { reactive } from 'vue'
import type { LessonSizes, VideoTrackKind, VideoTrackSize } from '@/types'
import { DEFAULT_VIDEO_TRACKS, VIDEO_TRACK_LABELS } from './downloadLayout'
import { formatBytes } from './format'

type Row = { tracks: LessonSizes['tracks']; loading: boolean }
export type SizeFetcher = (courseId: string, lessonId: string, tracks: VideoTrackKind[], signal: AbortSignal, refresh?: boolean) => Promise<LessonSizes>
const unavailable = (): VideoTrackSize => ({ status: 'unavailable', size: null })

export function selectedSize(tracks: LessonSizes['tracks'], selected: VideoTrackKind[]) {
  const values = [...new Set(selected)].map((kind) => tracks[kind])
  if (!values.length || values.some((value) => !value)) return { status: 'pending' as const, size: null }
  if (values.some((value) => value?.status === 'missing')) return { status: 'missing' as const, size: null }
  if (values.some((value) => value?.status !== 'ready' || !Number.isSafeInteger(value.size) || value.size! <= 0)) return { status: 'unavailable' as const, size: null }
  const size = values.reduce((total, value) => total + value!.size!, 0)
  return Number.isSafeInteger(size) ? { status: 'ready' as const, size } : { status: 'unavailable' as const, size: null }
}

export function createLessonSizeLoader(courseId: string, fetchSizes: SizeFetcher) {
  const rows = reactive<Record<string, Row>>({})
  const visible = new Set<string>()
  const running = new Map<string, AbortController>()
  const retries = new Set<string>()
  let selected: VideoTrackKind[] = [...DEFAULT_VIDEO_TRACKS]
  let enabled = true
  let disposed = false
  const row = (id: string) => rows[id] ?? (rows[id] = { tracks: {}, loading: false })
  const needed = (id: string) => selected.filter((kind) => !row(id).tracks[kind])

  function pump() {
    if (disposed || !enabled) return
    for (const id of visible) {
      if (running.size >= 2) break
      const requested = needed(id)
      if (running.has(id) || !requested.length) continue
      const controller = new AbortController()
      running.set(id, controller)
      row(id).loading = true
      const refresh = retries.delete(id)
      void fetchSizes(courseId, id, requested, AbortSignal.any([controller.signal, AbortSignal.timeout(30000)]), refresh)
        .then((response) => {
          if (disposed || controller.signal.aborted) return
          if (response.videoId !== id) throw new Error('录像大小响应不匹配')
          for (const kind of requested) {
            const value = response.tracks[kind]
            row(id).tracks[kind] = value?.status === 'ready' && Number.isSafeInteger(value.size) && value.size! > 0
              ? value : value?.status === 'missing' ? value : unavailable()
          }
        }).catch(() => {
          if (!disposed && !controller.signal.aborted) {
            for (const kind of requested) row(id).tracks[kind] = unavailable()
          }
        }).finally(() => {
          running.delete(id)
          if (!disposed) { row(id).loading = false; pump() }
        })
    }
  }
  function setVisible(id: string, value: boolean) {
    if (value) visible.add(id)
    else visible.delete(id)
    pump()
  }
  function setTracks(value: VideoTrackKind[]) { selected = [...new Set(value)]; pump() }
  function setEnabled(value: boolean) {
    enabled = value
    if (!enabled) running.forEach((controller) => controller.abort())
    else pump()
  }
  function retry(id: string) {
    if (running.has(id) || disposed) return
    for (const kind of selected) {
      if (row(id).tracks[kind]?.status !== 'ready') delete row(id).tracks[kind]
    }
    retries.add(id)
    visible.add(id)
    pump()
  }
  function summary(id: string, tracks: VideoTrackKind[]) {
    const result = selectedSize(rows[id]?.tracks ?? {}, tracks)
    const text = result.status === 'ready' ? formatBytes(result.size)
      : result.status === 'missing' ? '无所选画面'
        : result.status === 'unavailable' ? '重试大小' : rows[id]?.loading ? '读取中…' : '—'
    const title = tracks.map((kind) => {
      const value = rows[id]?.tracks[kind]
      return `${VIDEO_TRACK_LABELS[kind]}：${value?.status === 'ready' ? formatBytes(value.size) : value?.status === 'missing' ? '无此画面' : '尚未获取'}`
    }).join('\n')
    return { ...result, text, title }
  }
  function dispose() {
    disposed = true
    running.forEach((controller) => controller.abort())
    visible.clear()
  }
  return { rows, setVisible, setTracks, setEnabled, retry, summary, dispose }
}
