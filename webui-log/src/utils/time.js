export function isZeroTime(value) {
  return String(value || '').startsWith('0001-01-01')
}

export function formatDateTime(value) {
  if (!value || isZeroTime(value)) {
    return '-'
  }
  const date = new Date(value)
  if (!Number.isFinite(date.getTime())) {
    return String(value)
  }
  return date.toLocaleString('zh-CN', { hour12: false })
}

export function formatRelativeTime(value, options = {}) {
  const { olderFormat = 'datetime' } = options
  if (!value) {
    return '-'
  }
  const ts = new Date(value).getTime()
  if (!Number.isFinite(ts)) {
    return String(value)
  }
  const diffSeconds = Math.max(0, Math.floor((Date.now() - ts) / 1000))
  if (diffSeconds < 5) {
    return '刚刚'
  }
  if (diffSeconds < 60) {
    return `${diffSeconds}秒前`
  }
  if (diffSeconds < 3600) {
    return `${Math.floor(diffSeconds / 60)}分钟前`
  }
  if (diffSeconds < 86400) {
    return `${Math.floor(diffSeconds / 3600)}小时前`
  }
  if (olderFormat === 'date') {
    return new Date(value).toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit' })
  }
  return new Date(value).toLocaleString('zh-CN', { hour12: false })
}
