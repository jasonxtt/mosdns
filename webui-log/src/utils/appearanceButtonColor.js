const BUTTON_COLOR_STORAGE_KEY = 'mosdns-button-color-settings-v1'

const DEFAULT_BUTTON_COLOR_SETTINGS = {
  light: { mode: 'default', color: '#40d889' },
  dark: { mode: 'default', color: '#40d889' }
}

function normalizeTheme(theme) {
  return String(theme) === 'dark' ? 'dark' : 'light'
}

function normalizeHexColor(raw, fallback) {
  const value = String(raw || '').trim()
  const matched = value.match(/^#?([0-9a-fA-F]{6})$/)
  if (!matched) {
    return String(fallback || '#40d889').toLowerCase()
  }
  return `#${matched[1].toLowerCase()}`
}

function normalizeEntry(raw, theme) {
  const base = DEFAULT_BUTTON_COLOR_SETTINGS[theme] || DEFAULT_BUTTON_COLOR_SETTINGS.light
  const mode = String(raw?.mode || '').toLowerCase() === 'custom' ? 'custom' : 'default'
  const color = normalizeHexColor(raw?.color, base.color)
  if (mode === 'custom') {
    return { mode, color }
  }
  return { mode: 'default', color: base.color }
}

function clampChannel(value) {
  return Math.max(0, Math.min(255, Math.round(value)))
}

function parseHexColor(raw, fallback) {
  const normalized = normalizeHexColor(raw, fallback)
  const matched = normalized.match(/^#([0-9a-fA-F]{6})$/)
  if (!matched) {
    return { r: 64, g: 216, b: 137 }
  }
  const value = matched[1]
  return {
    r: parseInt(value.slice(0, 2), 16),
    g: parseInt(value.slice(2, 4), 16),
    b: parseInt(value.slice(4, 6), 16)
  }
}

function toHex({ r, g, b }) {
  return `#${[r, g, b].map((channel) => clampChannel(channel).toString(16).padStart(2, '0')).join('')}`
}

function lightenColor(raw, amount, fallback) {
  const color = parseHexColor(raw, fallback)
  return toHex({
    r: color.r + (255 - color.r) * amount,
    g: color.g + (255 - color.g) * amount,
    b: color.b + (255 - color.b) * amount
  })
}

function darkenColor(raw, amount, fallback) {
  const color = parseHexColor(raw, fallback)
  return toHex({
    r: color.r * (1 - amount),
    g: color.g * (1 - amount),
    b: color.b * (1 - amount)
  })
}

function toRgba(raw, alpha, fallback) {
  const color = parseHexColor(raw, fallback)
  return `rgba(${color.r}, ${color.g}, ${color.b}, ${alpha})`
}

export function normalizeButtonColorSettings(raw = {}) {
  return {
    light: normalizeEntry(raw.light, 'light'),
    dark: normalizeEntry(raw.dark, 'dark')
  }
}

export function getDefaultButtonColorSettings() {
  return {
    light: { ...DEFAULT_BUTTON_COLOR_SETTINGS.light },
    dark: { ...DEFAULT_BUTTON_COLOR_SETTINGS.dark }
  }
}

export function getEffectiveButtonColor(theme, settings) {
  const t = normalizeTheme(theme)
  const normalized = normalizeButtonColorSettings(settings)
  const entry = normalized[t]
  if (entry.mode === 'custom') {
    return entry.color
  }
  return DEFAULT_BUTTON_COLOR_SETTINGS[t].color
}

export function applyButtonColorForTheme(theme, settings) {
  const t = normalizeTheme(theme)
  const root = document.documentElement
  const color = getEffectiveButtonColor(t, settings)
  root.style.setProperty('--switch-on-bg-top', lightenColor(color, 0.16, DEFAULT_BUTTON_COLOR_SETTINGS[t].color))
  root.style.setProperty('--switch-on-bg', color)
  root.style.setProperty('--switch-on-border', darkenColor(color, 0.1, DEFAULT_BUTTON_COLOR_SETTINGS[t].color))
  root.style.setProperty('--switch-on-shadow', toRgba(color, 0.28, DEFAULT_BUTTON_COLOR_SETTINGS[t].color))
}

export function loadButtonColorSettingsFromStorage() {
  let raw = null
  try {
    raw = JSON.parse(localStorage.getItem(BUTTON_COLOR_STORAGE_KEY) || 'null')
  } catch {
    raw = null
  }
  return normalizeButtonColorSettings(raw || getDefaultButtonColorSettings())
}

export function saveButtonColorSettingsToStorage(settings) {
  const normalized = normalizeButtonColorSettings(settings)
  localStorage.setItem(BUTTON_COLOR_STORAGE_KEY, JSON.stringify(normalized))
}

export function normalizeUserHexButtonColor(raw, fallback) {
  return normalizeHexColor(raw, fallback)
}
