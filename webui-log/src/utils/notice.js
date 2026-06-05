export function showTopNotice(message, tone = 'success') {
  if (typeof window === 'undefined') {
    return
  }
  window.dispatchEvent(
    new CustomEvent('mosdns-top-notice', {
      detail: {
        message: String(message || ''),
        tone
      }
    })
  )
}

export function setError(message) {
  showTopNotice(message, 'error')
}

export function setSuccess(message) {
  showTopNotice(message, 'success')
}

export function clearTopNotice() {
  showTopNotice('', 'success')
}
