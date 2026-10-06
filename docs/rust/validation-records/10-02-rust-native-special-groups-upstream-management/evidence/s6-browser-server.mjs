import http from 'node:http'
import path from 'node:path'
import { readFile } from 'node:fs/promises'
import { appendFile } from 'node:fs/promises'

const apiPort = Number(process.env.NATIVE_API_PORT)
const listenPort = Number(process.env.BROWSER_SERVER_PORT)
const webRoot = path.resolve(process.env.WEB_ROOT || '')
const tracePath = process.env.TRACE_LOG
if (!apiPort || !listenPort || !process.env.WEB_ROOT || !tracePath) {
  throw new Error('NATIVE_API_PORT, BROWSER_SERVER_PORT, WEB_ROOT and TRACE_LOG are required')
}

const mime = new Map([
  ['.html', 'text/html; charset=utf-8'], ['.js', 'text/javascript; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'], ['.svg', 'image/svg+xml'], ['.png', 'image/png'],
  ['.ico', 'image/x-icon'], ['.woff2', 'font/woff2'], ['.json', 'application/json; charset=utf-8']
])

async function proxyApi(request, response) {
  const chunks = []
  for await (const chunk of request) chunks.push(chunk)
  const requestBody = Buffer.concat(chunks)
  const headers = { ...request.headers }
  delete headers.host
  delete headers.connection
  try {
    const upstream = await fetch(`http://127.0.0.1:${apiPort}${request.url}`, {
      method: request.method,
      headers,
      body: ['GET', 'HEAD'].includes(request.method) ? undefined : requestBody
    })
    const body = Buffer.from(await upstream.arrayBuffer())
    await appendFile(tracePath, `${JSON.stringify({
      time: new Date().toISOString(), method: request.method, path: request.url,
      request: requestBody.length ? requestBody.toString('utf8') : null,
      status: upstream.status, response: body.toString('utf8')
    })}\n`)
    const responseHeaders = Object.fromEntries(upstream.headers.entries())
    delete responseHeaders.connection
    delete responseHeaders['transfer-encoding']
    response.writeHead(upstream.status, responseHeaders)
    response.end(body)
  } catch (error) {
    response.writeHead(502, { 'content-type': 'text/plain; charset=utf-8' })
    response.end(String(error))
  }
}

const server = http.createServer(async (request, response) => {
  if (request.url.startsWith('/api/') || request.url.startsWith('/plugins/') || request.url.startsWith('/metrics')) {
    await proxyApi(request, response)
    return
  }
  let pathname
  try {
    pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
  } catch {
    response.writeHead(400).end('bad path')
    return
  }
  if (pathname === '/') pathname = '/log.html'
  const file = path.resolve(webRoot, `.${pathname}`)
  if (!file.startsWith(`${webRoot}${path.sep}`)) {
    response.writeHead(404).end('not found')
    return
  }
  try {
    const body = await readFile(file)
    response.writeHead(200, { 'content-type': mime.get(path.extname(file)) || 'application/octet-stream' })
    response.end(body)
  } catch {
    response.writeHead(404).end('not found')
  }
})

server.listen(listenPort, '127.0.0.1', () => process.stdout.write(`built UI listening on 127.0.0.1:${listenPort}\n`))
