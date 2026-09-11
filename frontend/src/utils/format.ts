export function formatBytes(value?: number | null, decimals = 1) {
  if (value === null || value === undefined || Number.isNaN(value)) return '大小未知'
  if (value === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  return `${(value / 1024 ** index).toFixed(index === 0 ? 0 : decimals)} ${units[index]}`
}

export function formatDate(value?: string | null, includeTime = false) {
  if (!value) return '未设置'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
    ...(includeTime ? { hour: '2-digit', minute: '2-digit', hour12: false } : {}),
  }).format(date)
}

export function initials(name?: string) {
  const normalized = name?.trim()
  if (!normalized) return 'CP'
  return Array.from(normalized).slice(-2).join('').toUpperCase()
}

export function safeFilename(value: string) {
  return value.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').replace(/[. ]+$/g, '').slice(0, 180) || 'download'
}
