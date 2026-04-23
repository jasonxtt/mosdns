function normalizeNumber(value: number | string | null | undefined): number {
  const num = Number(value ?? 0)
  return Number.isFinite(num) ? num : 0
}

export function formatCount(value: number | string | null | undefined): string {
  return Math.round(normalizeNumber(value)).toLocaleString('zh-CN')
}

export function formatLatencyMs(value: number | string | null | undefined, withUnit = true): string {
  const text = normalizeNumber(value).toFixed(2)
  return withUnit ? `${text} ms` : text
}
