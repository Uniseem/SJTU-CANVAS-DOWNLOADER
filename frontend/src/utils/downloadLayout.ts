import type { DownloadRequestItem } from '@/types'

export const DEFAULT_VIDEO_TRACKS = ['slides', 'teacher'] as const
export const VIDEO_TRACK_LABELS = { slides: '电脑屏幕', teacher: '教室摄像头', composite: '合成画面' } as const

// Every segment is a name, never a relative/absolute filesystem path.
export function safeSegment(value: string, limit = 70): string {
  const cleaned = value.normalize('NFC').replace(/[<>:"/\\|?*\u0000-\u001f\u007f]/g, '_')
    .trim().slice(0, limit).replace(/[. ]+$/g, '') || '未命名'
  return /^(con|prn|aux|nul|com[1-9¹²³]|lpt[1-9¹²³])(?:\.|$)/i.test(cleaned) ? `_${cleaned}` : cleaned
}

function shortId(value: string) {
  let hash = 2166136261
  for (const character of value) hash = Math.imul(hash ^ character.charCodeAt(0), 16777619)
  return (hash >>> 0).toString(16).padStart(8, '0')
}

function lessonDate(value?: string) {
  if (!value) return ''
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return ''
  const parts = new Intl.DateTimeFormat('sv-SE', {
    timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
  }).formatToParts(date)
  const part = (type: string) => parts.find((p) => p.type === type)?.value ?? ''
  return `${part('year')}-${part('month')}-${part('day')}_${part('hour')}-${part('minute')}`
}

export function downloadLayout(item: DownloadRequestItem, serverFilename: string) {
  const course = `${safeSegment(item.courseName || '课程', 48)} [${safeSegment(item.courseId || '未知', 20)}]`
  if (item.type === 'file') {
    const extension = serverFilename.match(/\.[a-z0-9]{1,10}$/i)?.[0] ?? ''
    const stem = extension ? serverFilename.slice(0, -extension.length) : serverFilename
    return { directories: [course, '课程文件'], filename: `${safeSegment(stem, 110)}${extension}` }
  }
  const label = VIDEO_TRACK_LABELS[item.track ?? 'teacher']
  const extension = serverFilename.match(/\.(mp4|webm|m4v|flv|mov|mkv)$/i)?.[0].toLowerCase() ?? '.mp4'
  const title = safeSegment(item.title || '课堂录像', 48)
  const lecture = [lessonDate(item.beginTime), `${title} [${shortId(item.id)}]`].filter(Boolean).join(' ')
  return { directories: [course, '课堂录像', lecture], filename: `${label}${extension}` }
}

// Serialize filename allocation across batches: concurrent views/duplicate
// requests in this tab must not choose the same still-empty output file.
let allocationQueue: Promise<unknown> = Promise.resolve()

export async function allocateDownloadFile(root: FileSystemDirectoryHandle, directories: string[], filename: string) {
  const allocation = allocationQueue.then(async () => {
    let directory = root
    for (const segment of directories) directory = await directory.getDirectoryHandle(segment, { create: true })
    const extension = filename.match(/\.[^.]+$/)?.[0] ?? ''
    const stem = extension ? filename.slice(0, -extension.length) : filename
    for (let index = 1; index <= 10000; index++) {
      const candidate = index === 1 ? filename : `${stem} (${index})${extension}`
      try {
        await directory.getFileHandle(candidate)
      } catch (error) {
        // A directory with this name also occupies the name. Never turn a
        // permissions/storage error into permission to overwrite something.
        if ((error as { name?: string }).name === 'TypeMismatchError') continue
        if ((error as { name?: string }).name !== 'NotFoundError') throw error
        const handle = await directory.getFileHandle(candidate, { create: true })
        return { handle, filename: candidate, relativePath: [...directories, candidate].join('/') }
      }
    }
    throw new Error('同名文件过多，请选择另一个保存文件夹')
  })
  allocationQueue = allocation.catch(() => undefined)
  return allocation
}
