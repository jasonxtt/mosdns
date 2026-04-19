import { reactive } from 'vue'

export const confirmState = reactive({
  open: false,
  title: '请确认操作',
  message: '',
  confirmText: '确认',
  cancelText: '取消',
  tone: 'primary'
})

let pendingResolve = null

export function openConfirm(message, options = {}) {
  return new Promise((resolve) => {
    if (pendingResolve) {
      pendingResolve(false)
      pendingResolve = null
    }

    confirmState.title = String(options.title || '请确认操作')
    confirmState.message = String(message || '')
    confirmState.confirmText = String(options.confirmText || '确认')
    confirmState.cancelText = String(options.cancelText || '取消')
    confirmState.tone = String(options.tone || 'primary')
    confirmState.open = true

    pendingResolve = resolve
  })
}

export function closeConfirm(result = false) {
  if (!confirmState.open) {
    return
  }
  confirmState.open = false
  const resolver = pendingResolve
  pendingResolve = null
  if (resolver) {
    resolver(Boolean(result))
  }
}
