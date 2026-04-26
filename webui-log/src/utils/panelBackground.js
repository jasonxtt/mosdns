const DEFAULT_SETTINGS = {
  mode: 'none',
  url: '',
  color: '#0f172a',
  lightColor: '#f8fafc',
  darkColor: '#0f172a',
  imageUrl: '',
  uploadId: '',
  opacity: 0.9,
  blur: 10
}

function clamp(value, min, max, fallback) {
  const num = Number(value)
  if (!Number.isFinite(num)) {
    return fallback
  }
  if (num < min) {
    return min
  }
  if (num > max) {
    return max
  }
  return num
}

function cssUrl(rawUrl) {
  const safe = String(rawUrl || '').replace(/"/g, '\\"')
  return `url("${safe}")`
}

function normalizeHexColor(rawColor, fallback = DEFAULT_SETTINGS.color) {
  const raw = String(rawColor || '').trim()
  const match = raw.match(/^#?([0-9a-fA-F]{6})$/)
  if (!match) {
    return fallback
  }
  return `#${match[1].toLowerCase()}`
}

function opacityToTransparency(opacity) {
  return Math.round(clamp(opacity, 0, 1, DEFAULT_SETTINGS.opacity) * 100)
}

function transparencyToOpacity(transparency) {
  return clamp(Number(transparency) / 100, 0, 1, DEFAULT_SETTINGS.opacity)
}

function preloadImage(url, timeoutMs = 10000) {
  return new Promise((resolve, reject) => {
    const img = new Image()
    let finished = false
    const timer = window.setTimeout(() => {
      if (finished) {
        return
      }
      finished = true
      reject(new Error('图片加载超时'))
    }, timeoutMs)

    img.onload = () => {
      if (finished) {
        return
      }
      finished = true
      window.clearTimeout(timer)
      resolve()
    }
    img.onerror = () => {
      if (finished) {
        return
      }
      finished = true
      window.clearTimeout(timer)
      reject(new Error('图片加载失败'))
    }
    img.src = String(url || '')
  })
}

function applyAppBackgroundClasses(enabled, transparency, blur) {
  const app = document.getElementById('app')
  if (!app) {
    return
  }
  Array.from(app.classList).forEach((className) => {
    if (className.startsWith('custom-background-') || className.startsWith('blur-intensity-')) {
      app.classList.remove(className)
    }
  })
  app.classList.toggle('custom-background', enabled)
  if (!enabled) {
    return
  }
  app.classList.add(`custom-background-${Math.round(clamp(transparency, 0, 100, 90))}`)
  app.classList.add(`blur-intensity-${Math.round(clamp(blur, 0, 40, DEFAULT_SETTINGS.blur))}`)
}

function applyCssVariables({ imageUrl, color, opacity, blur }) {
  const root = document.documentElement
  const hasImage = Boolean(imageUrl)
  const hasColor = Boolean(color)
  const clampedOpacity = clamp(opacity, 0, 1, DEFAULT_SETTINGS.opacity)
  const transparency = opacityToTransparency(clampedOpacity)
  const blurPx = Math.round(clamp(blur, 0, 40, DEFAULT_SETTINGS.blur))
  const primaryAlpha = clampedOpacity
  const primaryHoverAlpha = Math.min(clampedOpacity + 0.08, 1)
  const switchKnobAlpha = Math.max(clampedOpacity, 0.12)
  root.setAttribute('data-panel-bg-enabled', hasImage || hasColor ? '1' : '0')
  root.style.setProperty('--page-bg-image', hasImage ? cssUrl(imageUrl) : 'none')
  root.style.setProperty('--page-bg-solid-color', hasColor ? color : 'transparent')
  root.style.setProperty('--page-bg-mask-opacity', '0')
  root.style.setProperty('--panel-glass-opacity', String(clampedOpacity))
  root.style.setProperty('--panel-glass-blur', `${blurPx}px`)
  root.style.setProperty('--panel-glass-transparency', String(transparency))
  root.style.setProperty('--panel-primary-alpha', String(primaryAlpha))
  root.style.setProperty('--panel-primary-hover-alpha', String(primaryHoverAlpha))
  root.style.setProperty('--panel-switch-knob-alpha', String(switchKnobAlpha))
  applyAppBackgroundClasses(hasImage || hasColor, transparency, blurPx)
}

export function normalizePanelBackgroundSettings(raw = {}) {
  const modeRaw = String(raw.mode || '').trim().toLowerCase()
  const mode = ['none', 'url', 'upload', 'color'].includes(modeRaw) ? modeRaw : DEFAULT_SETTINGS.mode
  const url = String(raw.url || '').trim()
  const lightColor = normalizeHexColor(raw.light_color ?? raw.lightColor ?? raw.color, DEFAULT_SETTINGS.lightColor)
  const darkColor = normalizeHexColor(raw.dark_color ?? raw.darkColor ?? raw.color, DEFAULT_SETTINGS.darkColor)
  const themeKeyRaw = String(raw.theme_key ?? raw.themeKey ?? 'light').trim().toLowerCase()
  const themeKey = themeKeyRaw === 'dark' ? 'dark' : 'light'
  const color = themeKey === 'dark' ? darkColor : lightColor
  const imageUrl = String(raw.image_url || raw.imageUrl || '').trim()
  const uploadId = String(raw.upload_id || raw.uploadId || '').trim()
  const rawTransparency = Number(raw.transparency)
  const opacityFallback = Number.isFinite(rawTransparency) ? transparencyToOpacity(rawTransparency) : DEFAULT_SETTINGS.opacity
  const opacity = clamp(raw.opacity, 0, 1, opacityFallback)
  const blur = Math.round(clamp(raw.blur, 0, 40, DEFAULT_SETTINGS.blur))
  const activeImageUrl = mode === 'url' ? url : mode === 'upload' ? imageUrl : ''
  const activeColor = mode === 'color' ? color : ''

  return {
    mode,
    url,
    color,
    lightColor,
    darkColor,
    themeKey,
    imageUrl,
    uploadId,
    activeImageUrl,
    activeColor,
    opacity,
    transparency: opacityToTransparency(opacity),
    blur
  }
}

export async function previewPanelBackground(rawSettings, options = {}) {
  const settings = normalizePanelBackgroundSettings(rawSettings)
  if (!settings.activeImageUrl) {
    applyCssVariables({
      imageUrl: '',
      color: settings.activeColor,
      opacity: settings.opacity,
      blur: settings.blur
    })
    return { ok: true, settings }
  }

  try {
    await preloadImage(settings.activeImageUrl, options.timeoutMs || 10000)
    applyCssVariables({
      imageUrl: settings.activeImageUrl,
      color: '',
      opacity: settings.opacity,
      blur: settings.blur
    })
    return { ok: true, settings }
  } catch (error) {
    applyCssVariables({
      imageUrl: '',
      color: '',
      opacity: settings.opacity,
      blur: settings.blur
    })
    if (typeof options.onError === 'function') {
      options.onError(error)
    }
    return { ok: false, settings, error }
  }
}

export function getDefaultPanelBackgroundSettings() {
  return {
    ...DEFAULT_SETTINGS,
    transparency: opacityToTransparency(DEFAULT_SETTINGS.opacity)
  }
}

export { opacityToTransparency, transparencyToOpacity }
