import http from 'node:http'
import path from 'node:path'
import { mkdir, writeFile } from 'node:fs/promises'

const outputDirectory = path.resolve(process.env.OUTPUT_DIR || '')
const port = Number(process.env.CAPTURE_PORT)
if (!outputDirectory || !port) {
  throw new Error('OUTPUT_DIR and CAPTURE_PORT are required')
}

const page = `<!doctype html><meta charset="utf-8"><title>S7 evidence capture</title>
<h1>S7 browser evidence capture</h1>
<p>Save the current in-app browser screenshot into the isolated task evidence folder.</p>
<form method="post" action="/capture">
  <label for="image">JPEG screenshot (base64)</label>
  <textarea id="image" name="image" rows="8" cols="80" required></textarea>
  <button type="submit">Save S7 browser screenshot</button>
</form>`

const server = http.createServer(async (request, response) => {
  if (request.method === 'GET' && request.url === '/capture') {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    response.end(page)
    return
  }
  if (request.method === 'POST' && request.url === '/capture') {
    const chunks = []
    for await (const chunk of request) chunks.push(chunk)
    const form = new URLSearchParams(Buffer.concat(chunks).toString('utf8'))
    const encoded = form.get('image') || ''
    if (!/^[A-Za-z0-9+/]+={0,2}$/.test(encoded)) {
      response.writeHead(400, { 'content-type': 'text/plain; charset=utf-8' })
      response.end('invalid base64 screenshot')
      return
    }
    const bytes = Buffer.from(encoded, 'base64')
    const isJpeg = bytes.length > 4 && bytes[0] === 0xff && bytes[1] === 0xd8
      && bytes[bytes.length - 2] === 0xff && bytes[bytes.length - 1] === 0xd9
    if (!isJpeg) {
      response.writeHead(400, { 'content-type': 'text/plain; charset=utf-8' })
      response.end('expected a complete JPEG screenshot')
      return
    }
    await mkdir(outputDirectory, { recursive: true })
    const file = path.join(outputDirectory, 's7-audit-detail.jpg')
    await writeFile(file, bytes, { flag: 'wx' })
    response.writeHead(201, { 'content-type': 'text/plain; charset=utf-8' })
    response.end(`saved s7-audit-detail.jpg (${bytes.length} bytes)\n`)
    return
  }
  response.writeHead(404).end('not found')
})

server.listen(port, '127.0.0.1', () => {
  process.stdout.write(`S7 screenshot sink listening on 127.0.0.1:${port}\n`)
})
