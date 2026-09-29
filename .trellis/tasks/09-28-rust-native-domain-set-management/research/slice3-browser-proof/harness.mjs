// Isolated end-to-end proof for Slice 3 of the Rust-native local-rule editing
// workflow.
//
// It starts, all on loopback with probed free ports:
//   1. the real Rust-native host binary (`.trellis` fixture config + rule files),
//   2. the real maintained Vite dev server with MOSDNS_DEV_TARGET pointed at
//      that native HTTP port (and nothing else),
//   3. real headless Google Chrome, driven through the DevTools Protocol.
//
// It then opens `/`, navigates Rules -> 本地规则, edits the configured fixed
// tag `blocklist`, saves, and asserts the observable result through the real
// native API, the real rule file and a real DNS query. Failure states are
// injected in the browser transport (CDP Fetch), which is the "controlled GET
// transport failure" boundary allowed by the task design.
//
// Usage: node harness.mjs [--keep] [--output <path>]
// Requires: rust/target/debug/mosdns and webui-log/node_modules.

import { spawn } from 'node:child_process'
import { createSocket } from 'node:dgram'
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync
} from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '../../../../..')
const NATIVE_BINARY = join(REPO, 'rust/target/debug/mosdns')
const WEBUI = join(REPO, 'webui-log')
const CHROME =
  process.env.CHROME_BINARY ||
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'

const results = []
const logs = []
const pageErrors = []
let failure = null

function check(name, condition, detail) {
  const entry = { name, ok: Boolean(condition), detail: detail ?? null }
  results.push(entry)
  const mark = entry.ok ? 'PASS' : 'FAIL'
  console.log(`[${mark}] ${name}${detail === undefined ? '' : ` :: ${detail}`}`)
  return entry.ok
}

function equal(name, actual, expected) {
  return check(name, actual === expected, `actual=${JSON.stringify(actual)} expected=${JSON.stringify(expected)}`)
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms))

async function freePort(kind) {
  if (kind === 'udp') {
    const socket = createSocket('udp4')
    await new Promise((done, fail) => {
      socket.once('error', fail)
      socket.bind(0, '127.0.0.1', done)
    })
    const port = socket.address().port
    await new Promise((done) => socket.close(done))
    return port
  }
  const server = createServer()
  await new Promise((done, fail) => {
    server.once('error', fail)
    server.listen(0, '127.0.0.1', done)
  })
  const port = server.address().port
  await new Promise((done) => server.close(done))
  return port
}

async function udpBindable(port) {
  const socket = createSocket('udp4')
  try {
    await new Promise((done, fail) => {
      socket.once('error', fail)
      socket.bind(port, '127.0.0.1', done)
    })
    return true
  } catch {
    return false
  } finally {
    socket.close()
  }
}

async function waitForHttp(url, predicate, label, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  let last = 'no attempt'
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url)
      const text = await response.text()
      if (predicate(response, text)) {
        return text
      }
      last = `status=${response.status} body=${text.slice(0, 120)}`
    } catch (error) {
      last = String(error.message ?? error)
    }
    await sleep(200)
  }
  throw new Error(`${label} did not become ready: ${last}`)
}

function dnsQuery(id, labels) {
  const header = Buffer.alloc(12)
  header.writeUInt16BE(id, 0)
  header.writeUInt16BE(0x0100, 2)
  header.writeUInt16BE(1, 4)
  const parts = [header]
  for (const label of labels) {
    parts.push(Buffer.from([label.length]), Buffer.from(label, 'ascii'))
  }
  parts.push(Buffer.from([0, 0, 1, 0, 1]))
  return Buffer.concat(parts)
}

async function dnsRcode(port, labels) {
  const socket = createSocket('udp4')
  const query = dnsQuery(0x4242, labels)
  try {
    const response = await new Promise((done, fail) => {
      const timer = setTimeout(() => fail(new Error('DNS query timed out')), 4000)
      socket.once('message', (message) => {
        clearTimeout(timer)
        done(message)
      })
      socket.once('error', (error) => {
        clearTimeout(timer)
        fail(error)
      })
      socket.send(query, port, '127.0.0.1')
    })
    return response[3] & 0x0f
  } finally {
    socket.close()
  }
}

class Cdp {
  constructor(socket) {
    this.socket = socket
    this.nextId = 1
    this.pending = new Map()
    this.handlers = new Map()
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data)
      if (message.id && this.pending.has(message.id)) {
        const { resolve: done, reject: fail } = this.pending.get(message.id)
        this.pending.delete(message.id)
        if (message.error) {
          fail(new Error(`${message.error.message} (${JSON.stringify(message.error)})`))
        } else {
          done(message.result)
        }
        return
      }
      const handler = this.handlers.get(message.method)
      if (handler) {
        handler(message.params)
      }
    })
  }

  on(method, handler) {
    this.handlers.set(method, handler)
  }

  send(method, params = {}) {
    const id = this.nextId++
    return new Promise((done, fail) => {
      this.pending.set(id, { resolve: done, reject: fail })
      this.socket.send(JSON.stringify({ id, method, params }))
    })
  }

  async evaluate(expression) {
    const result = await this.send('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true
    })
    if (result.exceptionDetails) {
      throw new Error(`page evaluation failed: ${JSON.stringify(result.exceptionDetails)}`)
    }
    return result.result.value
  }
}

async function connectCdp(port) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()
      const page = targets.find((target) => target.type === 'page' && target.webSocketDebuggerUrl)
      if (page) {
        const socket = new WebSocket(page.webSocketDebuggerUrl)
        await new Promise((done, fail) => {
          socket.addEventListener('open', done, { once: true })
          socket.addEventListener('error', () => fail(new Error('CDP socket error')), { once: true })
        })
        return new Cdp(socket)
      }
    } catch {
      // Chrome is still starting.
    }
    await sleep(300)
  }
  throw new Error('could not attach to Chrome DevTools')
}

/** Waits until an in-page predicate returns true. */
async function waitFor(cdp, expression, label, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await cdp.evaluate(expression)) {
      return
    }
    await sleep(200)
  }
  throw new Error(`${label} (waited ${timeoutMs}ms)`)
}

/** Clicks one element, retrying while a fresh document is still rendering. */
async function clickUntil(cdp, selector, text, label) {
  const deadline = Date.now() + 20_000
  while (Date.now() < deadline) {
    if (await cdp.evaluate(CLICK_TEXT(selector, text))) {
      check(label, true)
      return
    }
    await sleep(200)
  }
  check(label, false, `could not click ${selector} ${text}`)
}

/** Records top notices in the page. Reinstalled after every navigation. */
async function installNoticeRecorder(cdp) {
  await cdp.evaluate(`(() => {
    if (!window.__slice3Recorder) {
      window.__slice3Recorder = true
      window.addEventListener('mosdns-top-notice', (event) => {
        const message = String(event?.detail?.message || '')
        if (message) { window.__slice3Notices.push({ message, tone: event?.detail?.tone }) }
      })
    }
    window.__slice3Notices = []
    return true
  })()`)
}

/** Opens Rules -> 本地规则 through the real navigation and waits for the editor. */
async function openLocalRules(cdp, phase) {
  await clickUntil(cdp, '.legacy-main-btn', '规则管理', `${phase}: Rules tab is clickable`)
  await sleep(300)
  await clickUntil(cdp, '.legacy-sub-btn', '本地规则', `${phase}: 本地规则 sub-tab is clickable`)
  await waitFor(
    cdp,
    `Boolean(document.querySelector('.list-editor')) && document.querySelectorAll('.list-btn').length > 0`,
    `${phase}: the local-rule page to render its editor`
  )
  await installNoticeRecorder(cdp)
  await sleep(300)
}

const SET_EDITOR = (text) => `(() => {
  const editor = document.querySelector('.list-editor')
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set
  setter.call(editor, ${JSON.stringify(text)})
  editor.dispatchEvent(new Event('input', { bubbles: true }))
  return editor.value
})()`

const CLICK_TEXT = (selector, text) => `(() => {
  const button = [...document.querySelectorAll(${JSON.stringify(selector)})]
    .find((item) => item.textContent.trim().startsWith(${JSON.stringify(text)}))
  if (!button) { return false }
  button.click()
  return true
})()`

const SNAPSHOT = `(() => ({
  editor: document.querySelector('.list-editor')?.value ?? null,
  status: document.querySelector('.list-status-inline')?.textContent ?? null,
  unsavedDots: [...document.querySelectorAll('.list-btn')].map((button) => ({
    name: button.textContent.trim(),
    dirty: Boolean(button.querySelector('.unsaved-dot'))
  })),
  notices: window.__slice3Notices ?? []
}))()`

async function main() {
  if (!existsSync(NATIVE_BINARY)) {
    throw new Error(`missing native binary: ${NATIVE_BINARY}`)
  }
  if (!existsSync(join(WEBUI, 'node_modules'))) {
    throw new Error(`missing ${WEBUI}/node_modules (run npm ci)`)
  }

  const dnsPort = await freePort('udp')
  const apiPort = await freePort('tcp')
  const vitePort = await freePort('tcp')
  const chromePort = await freePort('tcp')
  const work = join(tmpdir(), `mosdns-slice3-${process.pid}`)
  rmSync(work, { recursive: true, force: true })
  mkdirSync(join(work, 'rules'), { recursive: true })

  // Vite's `closeBundle` stamps the tracked generated assets even when it runs
  // as a dev server, so the dev server runs from a disposable copy of the Vue
  // sources plus a copy of `coremain/www`. The main worktree is never written;
  // that is asserted after the run.
  const source = join(work, 'source')
  const sourceWebui = join(source, 'webui-log')
  cpSync(join(REPO, 'webui-log'), sourceWebui, {
    recursive: true,
    filter: (entry) => !entry.includes('node_modules')
  })
  symlinkSync(join(REPO, 'webui-log/node_modules'), join(sourceWebui, 'node_modules'), 'dir')
  cpSync(join(REPO, 'coremain/www'), join(source, 'coremain/www'), { recursive: true })
  const protectedAssets = [
    join(REPO, 'coremain/www/log.html'),
    join(REPO, 'coremain/www/assets/vue-log/index.html')
  ]
  const protectedBefore = protectedAssets.map((path) => readFileSync(path, 'utf8'))

  const config = readFileSync(join(HERE, 'config.template.yaml'), 'utf8')
    .replaceAll('__API_PORT__', String(apiPort))
    .replaceAll('__DNS_PORT__', String(dnsPort))
  writeFileSync(join(work, 'config.yaml'), config)
  writeFileSync(join(work, 'rules/blocklist.txt'), readFileSync(join(HERE, 'seed-blocklist.txt')))
  writeFileSync(join(work, 'rules/whitelist.txt'), readFileSync(join(HERE, 'seed-whitelist.txt')))

  const children = []
  const spawnLogged = (name, command, args, options = {}) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: { ...process.env, ...options.env },
      stdio: ['ignore', 'pipe', 'pipe']
    })
    child.stdout.on('data', (data) => logs.push({ name, stream: 'stdout', data: String(data) }))
    child.stderr.on('data', (data) => logs.push({ name, stream: 'stderr', data: String(data) }))
    children.push({ name, child })
    return child
  }
  const stopAll = async () => {
    for (const { child } of children) {
      if (!child.killed) {
        child.kill('SIGTERM')
      }
    }
    await sleep(600)
    for (const { child } of children) {
      if (!child.killed) {
        child.kill('SIGKILL')
      }
    }
  }

  const nativeHttp = `http://127.0.0.1:${apiPort}`
  const viteHttp = `http://127.0.0.1:${vitePort}`
  let cdp = null
  try {
    // 1. Native host.
    const native = spawnLogged('native', NATIVE_BINARY, ['start', '-c', join(work, 'config.yaml')])
    await waitForHttp(`${nativeHttp}/api/v1/special-groups`, (response) => response.ok, 'native HTTP', 30_000)
    check('native host serves the scoped API', true, nativeHttp)

    // 2. Vite with an explicit isolated proxy target.
    spawnLogged('vite', 'npm', ['run', 'dev', '--', '--port', String(vitePort), '--strictPort', '--host', '127.0.0.1'], {
      cwd: sourceWebui,
      env: { MOSDNS_DEV_TARGET: nativeHttp }
    })
    await waitForHttp(viteHttp, (_response, text) => text.includes('<div id="app"'), 'vite dev server', 90_000)

    // 3. Proxy destination proof: a native-only route answered through Vite.
    const proxiedGroups = await (await fetch(`${viteHttp}/api/v1/special-groups`)).text()
    equal('vite proxies /api/v1/special-groups to the isolated native host', proxiedGroups, '[]\n')
    const proxiedShow = await (await fetch(`${viteHttp}/plugins/blocklist/show?limit=10000`)).text()
    equal('vite proxies /plugins to the isolated native host', proxiedShow, 'seed-blocked.example\n')
    const directShow = await (await fetch(`${nativeHttp}/plugins/blocklist/show?limit=10000`)).text()
    equal('the direct native port answers the same content', directShow, proxiedShow)

    // 4. Real headless Chrome.
    spawnLogged('chrome', CHROME, [
      '--headless=new',
      `--remote-debugging-port=${chromePort}`,
      `--user-data-dir=${join(work, 'chrome-profile')}`,
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-gpu',
      '--disable-dev-shm-usage',
      'about:blank'
    ])
    cdp = await connectCdp(chromePort)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    cdp.on('Page.loadEventFired', () => {})

    cdp.on('Runtime.exceptionThrown', (params) => {
      pageErrors.push(String(params?.exceptionDetails?.text ?? 'exception') + ' ' + String(params?.exceptionDetails?.exception?.description ?? ''))
    })
    cdp.on('Runtime.consoleAPICalled', (params) => {
      if (params?.type === 'error') {
        pageErrors.push((params.args ?? []).map((arg) => String(arg.value ?? arg.description ?? '')).join(' '))
      }
    })

    await cdp.send('Page.navigate', { url: `${viteHttp}/` })
    await openLocalRules(cdp, 'initial')

    // 5. Select the configured fixed tag.
    check('黑名单 profile button is present and clickable', await cdp.evaluate(CLICK_TEXT('.list-btn', '黑名单')))
    await sleep(600)
    let snapshot = await cdp.evaluate(SNAPSHOT)
    equal('the page loaded the configured tag through the Vite proxy', snapshot.editor, 'seed-blocked.example\n')

    // 6. Edit and save through the real page; assert file + DNS effect.
    await cdp.evaluate(`window.__slice3Notices = []`)
    await cdp.evaluate(SET_EDITOR('seed-blocked.example\nui-added.example\n'))
    check('saving the edited list', await cdp.evaluate(CLICK_TEXT('.save-list-btn', '保存')))
    await sleep(800)
    snapshot = await cdp.evaluate(SNAPSHOT)
    equal('the file contains the edited rule', readFileSync(join(work, 'rules/blocklist.txt'), 'utf8'), 'seed-blocked.example\nui-added.example\n')
    equal('the next real UDP query reflects the saved rule', await dnsRcode(dnsPort, ['ui-added', 'example']), 3)
    equal('a previously blocked rule stays blocked', await dnsRcode(dnsPort, ['seed-blocked', 'example']), 3)
    equal('an unrelated name is not blocked', await dnsRcode(dnsPort, ['other', 'example']), 0)
    check('the save is reported as confirmed', snapshot.notices.some((notice) => notice.message.includes('已保存 1 个列表')), JSON.stringify(snapshot.notices))

    // 7. Refresh retains the list.
    await cdp.send('Page.reload', { ignoreCache: true })
    await sleep(1200)
    await openLocalRules(cdp, 'refresh')
    await clickUntil(cdp, '.list-btn', '黑名单', 'refresh: 黑名单 profile is clickable')
    await sleep(600)
    snapshot = await cdp.evaluate(SNAPSHOT)
    equal('a page refresh retains the saved list', snapshot.editor, 'seed-blocked.example\nui-added.example\n')

    // 8. A skipped invalid rule is only shown as saved after the canonical read.
    await cdp.evaluate(`window.__slice3Notices = []`)
    await cdp.evaluate(SET_EDITOR('seed-blocked.example\nregexp:[\nui-second.example\n'))
    await cdp.evaluate(CLICK_TEXT('.save-list-btn', '保存'))
    await sleep(800)
    snapshot = await cdp.evaluate(SNAPSHOT)
    equal('a skipped rule is not presented as canonical', snapshot.editor, 'seed-blocked.example\nui-second.example\n')
    equal('the file holds only accepted rules', readFileSync(join(work, 'rules/blocklist.txt'), 'utf8'), 'seed-blocked.example\nui-second.example\n')
    check('the skipped rule is reported to the user', snapshot.notices.some((notice) => notice.message.includes('已按服务器内容调整')), JSON.stringify(snapshot.notices))

    // 9. Restart the native host; the accepted rules survive.
    native.kill('SIGTERM')
    await sleep(800)
    spawnLogged('native-restart', NATIVE_BINARY, ['start', '-c', join(work, 'config.yaml')])
    await waitForHttp(`${nativeHttp}/api/v1/special-groups`, (response) => response.ok, 'restarted native HTTP', 30_000)
    equal('a restarted host serves the same accepted rules', await (await fetch(`${nativeHttp}/plugins/blocklist/show?limit=10000`)).text(), 'seed-blocked.example\nui-second.example\n')
    equal('a restarted host keeps the DNS effect', await dnsRcode(dnsPort, ['ui-second', 'example']), 3)

    // 10. A failed POST shows an error and keeps the draft.
    await cdp.send('Page.reload', { ignoreCache: true })
    await sleep(1200)
    await openLocalRules(cdp, 'post-failure')
    await clickUntil(cdp, '.list-btn', '黑名单', 'post-failure: 黑名单 profile is clickable')
    await sleep(600)
    await cdp.evaluate(`window.__slice3Notices = []`)
    const beforePostFailure = readFileSync(join(work, 'rules/blocklist.txt'), 'utf8')
    // Fail every POST while this flag is set.
    const postFault = { armed: true }
    await cdp.send('Fetch.enable', { patterns: [{ urlPattern: '*/plugins/*/post' }] })
    cdp.on('Fetch.requestPaused', async (params) => {
      if (postFault.armed) {
        await cdp.send('Fetch.failRequest', { requestId: params.requestId, errorReason: 'ConnectionFailed' })
      } else {
        await cdp.send('Fetch.continueRequest', { requestId: params.requestId })
      }
    })
    await cdp.evaluate(SET_EDITOR('seed-blocked.example\nui-second.example\nui-post-fail.example\n'))
    await cdp.evaluate(CLICK_TEXT('.save-list-btn', '保存'))
    await sleep(900)
    snapshot = await cdp.evaluate(SNAPSHOT)
    equal('a failed POST keeps the draft', snapshot.editor, 'seed-blocked.example\nui-second.example\nui-post-fail.example\n')
    equal('a failed POST does not touch the file', readFileSync(join(work, 'rules/blocklist.txt'), 'utf8'), beforePostFailure)
    check('a failed POST is reported', snapshot.notices.some((notice) => notice.message.includes('保存失败')), JSON.stringify(snapshot.notices))
    postFault.armed = false
    await cdp.send('Fetch.disable')

    // 11. POST 200 with a failing canonical read leaves that tag unconfirmed
    //     while another dirty tag still reconciles independently.
    await cdp.send('Page.reload', { ignoreCache: true })
    await sleep(1200)
    await openLocalRules(cdp, 'unconfirmed')
    await clickUntil(cdp, '.list-btn', '黑名单', 'unconfirmed: 黑名单 profile is clickable')
    await sleep(600)
    await cdp.evaluate(`window.__slice3Notices = []`)
    await cdp.evaluate(SET_EDITOR('seed-blocked.example\nui-second.example\nui-unconfirmed.example\n'))
    // Edit the second tag too, so both are dirty.
    await clickUntil(cdp, '.list-btn', '白名单', 'unconfirmed: 白名单 profile is clickable')
    await sleep(500)
    await cdp.evaluate(SET_EDITOR('seed-white.example\nui-white.example\n'))
    await clickUntil(cdp, '.list-btn', '黑名单', 'unconfirmed: 黑名单 profile re-selected')
    await sleep(500)

    // Intercept only the canonical read that follows the blocklist POST.
    await cdp.send('Fetch.enable', { patterns: [{ urlPattern: '*/plugins/*' }] })
    let blocklistPostSeen = false
    let armed = true
    cdp.on('Fetch.requestPaused', async (params) => {
      const url = params.request.url
      const isPost = url.includes('/post')
      const isBlocklistShow = url.includes('/plugins/blocklist/show')
      if (armed && isPost && url.includes('/plugins/blocklist/post')) {
        blocklistPostSeen = true
        await cdp.send('Fetch.continueRequest', { requestId: params.requestId })
        return
      }
      if (armed && blocklistPostSeen && isBlocklistShow) {
        blocklistPostSeen = false
        await cdp.send('Fetch.failRequest', { requestId: params.requestId, errorReason: 'ConnectionFailed' })
        return
      }
      await cdp.send('Fetch.continueRequest', { requestId: params.requestId })
    })
    await cdp.evaluate(CLICK_TEXT('.save-list-btn', '保存'))
    await sleep(1200)
    snapshot = await cdp.evaluate(SNAPSHOT)
    check(
      'an unconfirmed tag keeps a recoverable draft',
      snapshot.editor.includes('ui-unconfirmed.example'),
      snapshot.editor
    )
    check(
      'an unconfirmed tag is reported as unconfirmed, not saved',
      snapshot.notices.some((notice) => notice.message.includes('当前内容未确认')),
      JSON.stringify(snapshot.notices)
    )
    check(
      'the confirmed count excludes the unconfirmed tag',
      snapshot.notices.some((notice) => notice.message.includes('已保存 1 个列表')),
      JSON.stringify(snapshot.notices)
    )
    equal(
      'the same POST still committed the file',
      readFileSync(join(work, 'rules/blocklist.txt'), 'utf8'),
      'seed-blocked.example\nui-second.example\nui-unconfirmed.example\n'
    )
    equal(
      'the other dirty tag reconciled independently',
      readFileSync(join(work, 'rules/whitelist.txt'), 'utf8'),
      'seed-white.example\nui-white.example\n'
    )
    const uncertainDots = snapshot.unsavedDots.filter((item) => item.name.startsWith('黑名单') && item.dirty)
    check('the unconfirmed tag is visibly marked', uncertainDots.length === 1, JSON.stringify(snapshot.unsavedDots))

    // 12. A later save retries the canonical read and reconciles.
    armed = false
    await cdp.send('Fetch.disable')
    await cdp.evaluate(`window.__slice3Notices = []`)
    await cdp.evaluate(CLICK_TEXT('.save-list-btn', '保存'))
    await sleep(1200)
    snapshot = await cdp.evaluate(SNAPSHOT)
    check(
      'a later save reconciles the uncertain tag',
      !snapshot.notices.some((notice) => notice.message.includes('当前内容未确认')),
      JSON.stringify(snapshot.notices)
    )
    const remainingDots = snapshot.unsavedDots.filter((item) => item.name.startsWith('黑名单') && item.dirty)
    check('the reconciled tag is no longer marked dirty', remainingDots.length === 0, JSON.stringify(snapshot.unsavedDots))

    check(
      'the main worktree generated assets are untouched',
      protectedAssets.every((path, index) => readFileSync(path, 'utf8') === protectedBefore[index]),
      protectedAssets.map((path) => path.replace(`${REPO}/`, '')).join(', ')
    )

    await stopAll()
    // Every isolated port must be free again after cleanup.
    check('the DNS port is released after shutdown', await udpBindable(dnsPort), String(dnsPort))
    const listener = createServer()
    await new Promise((done, fail) => {
      listener.once('error', fail)
      listener.listen(apiPort, '127.0.0.1', done)
    })
    check('the management port is released after shutdown', true, String(apiPort))
    await new Promise((done) => listener.close(done))
  } catch (error) {
    failure = String(error?.stack ?? error)
    console.error(`[ERROR] ${failure}`)
    await stopAll()
  } finally {
    if (!process.argv.includes('--keep')) {
      rmSync(work, { recursive: true, force: true })
    }
    const outputIndex = process.argv.indexOf('--output')
    const outputPath = outputIndex >= 0 ? process.argv[outputIndex + 1] : join(HERE, 'evidence.json')
    const payload = {
      recordedAt: new Date().toISOString(),
      nativeBinary: NATIVE_BINARY,
      viteProxyTarget: nativeHttp,
      viteUrl: viteHttp,
      ports: { dns: dnsPort, api: apiPort, vite: vitePort, chromeDebug: chromePort },
      fixture: { work: process.argv.includes('--keep') ? work : '(removed)', config },
      checks: results,
      passed: results.filter((item) => item.ok).length,
      failed: results.filter((item) => !item.ok).length,
      failure,
      pageErrors,
      logs
    }
    writeFileSync(outputPath, `${JSON.stringify(payload, null, 2)}\n`)
    console.log(`\nchecks: ${payload.passed} passed, ${payload.failed} failed`)
    console.log(`evidence: ${outputPath}`)
    if (failure || payload.failed > 0) {
      process.exitCode = 1
    }
  }
}

await main()
