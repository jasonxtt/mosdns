const TEXT_COLOR_STORAGE_KEY = 'mosdns-text-color-settings-v1'

const DEFAULT_TEXT_COLOR_SETTINGS = {
  light: { mode: 'default', color: '#1e252b' },
  dark: { mode: 'default', color: '#f8fafc' }
}

function normalizeTheme(theme) {
  return String(theme) === 'dark' ? 'dark' : 'light'
}

function normalizeHexColor(raw, fallback) {
  const value = String(raw || '').trim()
  const matched = value.match(/^#?([0-9a-fA-F]{6})$/)
  if (!matched) {
    return String(fallback || '#000000').toLowerCase()
  }
  return `#${matched[1].toLowerCase()}`
}

function normalizeEntry(raw, theme) {
  const base = DEFAULT_TEXT_COLOR_SETTINGS[theme] || DEFAULT_TEXT_COLOR_SETTINGS.light
  const mode = String(raw?.mode || '').toLowerCase() === 'custom' ? 'custom' : 'default'
  const color = normalizeHexColor(raw?.color, base.color)
  if (mode === 'custom') {
    return { mode, color }
  }
  return { mode: 'default', color: base.color }
}

export function normalizeTextColorSettings(raw = {}) {
  return {
    light: normalizeEntry(raw.light, 'light'),
    dark: normalizeEntry(raw.dark, 'dark')
  }
}

export function getDefaultTextColorSettings() {
  return {
    light: { ...DEFAULT_TEXT_COLOR_SETTINGS.light },
    dark: { ...DEFAULT_TEXT_COLOR_SETTINGS.dark }
  }
}

export function toMutedTextColor(hex, theme) {
  const normalized = normalizeHexColor(hex, DEFAULT_TEXT_COLOR_SETTINGS[normalizeTheme(theme)].color)
  const match = normalized.match(/^#([0-9a-fA-F]{6})$/)
  if (!match) {
    return theme === 'dark' ? 'rgba(248, 250, 252, 0.78)' : 'rgba(30, 37, 43, 0.74)'
  }
  const v = match[1]
  const r = parseInt(v.slice(0, 2), 16)
  const g = parseInt(v.slice(2, 4), 16)
  const b = parseInt(v.slice(4, 6), 16)
  const alpha = normalizeTheme(theme) === 'dark' ? 0.78 : 0.74
  return `rgba(${r}, ${g}, ${b}, ${alpha})`
}

export function getEffectiveTextColor(theme, settings) {
  const t = normalizeTheme(theme)
  const normalized = normalizeTextColorSettings(settings)
  const entry = normalized[t]
  if (entry.mode === 'custom') {
    return entry.color
  }
  return DEFAULT_TEXT_COLOR_SETTINGS[t].color
}

export function applyTextColorForTheme(theme, settings) {
  const t = normalizeTheme(theme)
  const normalized = normalizeTextColorSettings(settings)
  const root = document.documentElement
  const entry = normalized[t]
  if (entry.mode !== 'custom') {
    root.style.removeProperty('--ink-0')
    root.style.removeProperty('--ink-1')
    root.removeAttribute('data-text-color-custom')
    root.removeAttribute('data-text-color-theme')
    return
  }

  root.style.setProperty('--ink-0', entry.color)
  root.style.setProperty('--ink-1', toMutedTextColor(entry.color, t))
  root.setAttribute('data-text-color-custom', '1')
  root.setAttribute('data-text-color-theme', t)
}

export function loadTextColorSettingsFromStorage() {
  let raw = null
  try {
    raw = JSON.parse(localStorage.getItem(TEXT_COLOR_STORAGE_KEY) || 'null')
  } catch {
    raw = null
  }
  return normalizeTextColorSettings(raw || getDefaultTextColorSettings())
}

export function saveTextColorSettingsToStorage(settings) {
  const normalized = normalizeTextColorSettings(settings)
  localStorage.setItem(TEXT_COLOR_STORAGE_KEY, JSON.stringify(normalized))
}

export function normalizeUserHexColor(raw, fallback) {
  return normalizeHexColor(raw, fallback)
}
