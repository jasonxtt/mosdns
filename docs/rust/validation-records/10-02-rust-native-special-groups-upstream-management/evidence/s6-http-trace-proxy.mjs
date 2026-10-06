import http from 'node:http'
import { appendFile } from 'node:fs/promises'

const host = '127.0.0.1'
const apiPort = Number(process.env.NATIVE_API_PORT)
const listenPort = Number(process.env.TRACE_PROXY_PORT)
const logPath = process.env.TRACE_LOG
if (!apiPort || !listenPort || !logPath) throw new Error('NATIVE_API_PORT, TRACE_PROXY_PORT and TRACE_LOG are required')

const server = http.createServer(async (incoming, outgoing) => {
  const chunks = []
  for await (const chunk of incoming) chunks.push(chunk)
  const requestBody = Buffer.concat(chunks)
  try {
    const headers = { ...incoming.headers }
    delete headers.host
    delete headers.connection
    const response = await fetch(`http://${host}:${apiPort}${incoming.url}`, {
      method: incoming.method,
      headers,
      body: ['GET', 'HEAD'].includes(incoming.method) ? undefined : requestBody
    })
    const responseBody = Buffer.from(await response.arrayBuffer())
    const record = {
      time: new Date().toISOString(),
      method: incoming.method,
      path: incoming.url,
      request: requestBody.length ? requestBody.toString('utf8') : null,
      status: response.status,
      response: responseBody.toString('utf8')
    }
    await appendFile(logPath, `${JSON.stringify(record)}\n`)
    const responseHeaders = Object.fromEntries(response.headers.entries())
    delete responseHeaders.connection
    delete responseHeaders['transfer-encoding']
    outgoing.writeHead(response.status, responseHeaders)
    outgoing.end(responseBody)
  } catch (error) {
    outgoing.writeHead(502, { 'content-type': 'text/plain' })
    outgoing.end(String(error))
  }
})

server.listen(listenPort, host, () => process.stdout.write(`trace proxy listening on ${host}:${listenPort}\n`))
