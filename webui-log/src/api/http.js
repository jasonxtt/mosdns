async function request(url, options = {}) {
  const response = await fetch(url, options)
  if (!response.ok) {
    let message = `HTTP ${response.status} ${response.statusText}`
    try {
      const data = await response.json()
      if (data?.error) {
        message = data.error
      }
    } catch {
      try {
        const text = await response.text()
        if (text) {
          message = text
        }
      } catch {
        // ignore
      }
    }
    throw new Error(message)
  }
  return response
}

export async function getJSON(url) {
  const response = await request(url)
  return response.json()
}

export async function getText(url) {
  const response = await request(url)
  return response.text()
}

export async function postJSON(url, body) {
  const response = await request(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  })
  const contentType = response.headers.get('content-type') || ''
  if (contentType.includes('application/json')) {
    return response.json()
  }
  return response.text()
}

export async function putJSON(url, body) {
  const response = await request(url, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  })
  const contentType = response.headers.get('content-type') || ''
  if (contentType.includes('application/json')) {
    return response.json()
  }
  return response.text()
}

export async function deleteRequest(url) {
  const response = await request(url, { method: 'DELETE' })
  if (response.status === 204) {
    return null
  }
  const contentType = response.headers.get('content-type') || ''
  if (contentType.includes('application/json')) {
    return response.json()
  }
  return response.text()
}
