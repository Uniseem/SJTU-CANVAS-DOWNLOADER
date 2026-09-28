import type { Course, DownloadInfo, DownloadStatus, Track } from '@shared/protocol';

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function formatBytes(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value) || value < 0) {
    return '—';
  }
  let size = value;
  let unit = 0;
  while (size >= 1000 && unit < UNITS.length - 1) {
    size /= 1024;
    unit += 1;
  }
  const digits = unit === 0 ? 0 : size >= 100 ? 0 : size >= 10 ? 1 : 2;
  return `${size.toFixed(digits)} ${UNITS[unit]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatEta(download: DownloadInfo): string | null {
  if (download.status !== 'downloading' || !download.total || download.speed <= 0) {
    return null;
  }
  const seconds = Math.max(0, Math.round((download.total - download.received) / download.speed));
  if (seconds < 60) {
    return `${seconds} 秒`;
  }
  if (seconds < 3600) {
    return `${Math.round(seconds / 60)} 分钟`;
  }
  return `${Math.floor(seconds / 3600)} 小时 ${Math.round((seconds % 3600) / 60)} 分钟`;
}

const WEEKDAYS = ['日', '一', '二', '三', '四', '五', '六'];

/** "2026-09-01 08:00:00" and "2026-09-01 09:40:00" → "9月1日 周一" and "08:00–09:40". */
export function formatLessonTime(begin: string, end: string): { date: string; time: string } {
  const start = parseLocal(begin);
  const finish = parseLocal(end);
  if (!start) {
    return { date: begin || '', time: '' };
  }
  const date = `${start.getMonth() + 1}月${start.getDate()}日 周${WEEKDAYS[start.getDay()]}`;
  const clock = (value: Date): string => `${pad(value.getHours())}:${pad(value.getMinutes())}`;
  const time = finish ? `${clock(start)}–${clock(finish)}` : clock(start);
  return { date, time };
}

export function lessonYear(begin: string): number | null {
  return parseLocal(begin)?.getFullYear() ?? null;
}

/** Times from the school ("2026-09-01 08:00:00", China time) and ISO strings. */
function parseLocal(value: string | null | undefined): Date | null {
  if (!value) {
    return null;
  }
  const school = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/.exec(value);
  if (school && !/[Zz]|[+-]\d{2}:?\d{2}$/.test(value)) {
    const [, year, month, day, hour, minute] = school;
    return new Date(Number(year), Number(month) - 1, Number(day), Number(hour), Number(minute));
  }
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

export function formatDateTime(value: string | null | undefined): string {
  const date = parseLocal(value);
  if (!date) {
    return '';
  }
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function formatDate(value: string | null | undefined): string {
  const date = parseLocal(value);
  return date ? `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` : '';
}

function pad(value: number): string {
  return value < 10 ? `0${value}` : String(value);
}

export const TRACK_LABELS: Record<Track, string> = {
  slides: '电脑屏幕',
  teacher: '教室摄像头',
  composite: '合成画面',
};

export function trackLabel(track: Track | string | null | undefined): string {
  return (track && TRACK_LABELS[track as Track]) || '视频';
}

export const STATUS_LABELS: Record<DownloadStatus, string> = {
  queued: '等待中',
  downloading: '下载中',
  paused: '已暂停',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
};

export function isUnfinished(status: DownloadStatus): boolean {
  return status === 'queued' || status === 'downloading' || status === 'paused';
}

export function isCurrentCourse(course: Course): boolean {
  return course.enrollment_state !== 'completed';
}

export function courseSubtitle(course: Course): string {
  return [course.course_code, course.term, course.teacher].filter((value) => value && value.trim()).join(' · ');
}

export function fileKind(file: { content_type?: string; filename: string }): string {
  const extension = file.filename.split('.').pop()?.toLowerCase() ?? '';
  const type = file.content_type?.toLowerCase() ?? '';
  if (type.includes('pdf') || extension === 'pdf') return 'PDF';
  if (/presentation|powerpoint/.test(type) || /^pptx?$/.test(extension)) return '幻灯片';
  if (/msword|wordprocessing/.test(type) || /^docx?$/.test(extension)) return '文档';
  if (/spreadsheet|excel/.test(type) || /^xlsx?$/.test(extension)) return '表格';
  if (type.startsWith('video/') || /^(mp4|mkv|mov|avi|flv)$/.test(extension)) return '视频';
  if (type.startsWith('audio/') || /^(mp3|m4a|wav)$/.test(extension)) return '音频';
  if (type.startsWith('image/') || /^(png|jpe?g|gif|webp|svg)$/.test(extension)) return '图片';
  if (/zip|rar|7z|tar|gz/.test(type) || /^(zip|rar|7z|tar|gz)$/.test(extension)) return '压缩包';
  return extension ? extension.toUpperCase() : '文件';
}

/** Runs `worker` over `items` with at most `limit` in flight. */
export async function runPool<T>(items: T[], limit: number, worker: (item: T) => Promise<void>): Promise<void> {
  const queue = [...items];
  const runners = Array.from({ length: Math.min(limit, queue.length) }, async () => {
    while (queue.length > 0) {
      const item = queue.shift()!;
      await worker(item);
    }
  });
  await Promise.all(runners);
}
