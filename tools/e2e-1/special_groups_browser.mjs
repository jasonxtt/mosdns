// Namespace-local, actual Vue interactions. Uses the VM's cached Playwright.
import fs from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
const require = createRequire(import.meta.url)
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || '/root/mosdns-rust-webui-20261004/browser-tools/node_modules/playwright')
const [root, role] = process.argv.slice(2)
const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
const results = []
try {
  for (const shell of role === 'native' ? ['/', '/log'] : ['/']) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } })
    const pageErrors = []
    page.on('pageerror', error => pageErrors.push(error.message))
    page.on('dialog', dialog => dialog.accept())
    await page.goto(`http://127.0.0.1:9099${shell}`)
    if (shell === '/log') await page.getByRole('button', { name: '系统设置', exact: true }).click()
    await page.getByRole('button', { name: '上游设置', exact: true }).click()
    await page.locator('.special-groups-summary').getByRole('button', { name: '管理', exact: true }).click()
    await page.getByRole('button', { name: '新增专属分流组', exact: true }).click()
    const editor = page.locator('.special-group-modal-card')
    const name = `browser-${role}-${shell === '/' ? 'main' : 'log'}`
    await page.locator('#special-group-name-vue').fill(name)
    await editor.locator('.special-source-add-row select').first().selectOption('normal')
    await editor.locator('.special-source-add-row').getByRole('button', { name: '＋ 添加', exact: true }).click()
    await editor.locator('.special-owned-section').getByRole('button', { name: '＋ 新增上游', exact: true }).click()
    const upstream = page.locator('.upstream-editor-modal-card')
    await upstream.getByPlaceholder('例如 cmcc_dns_1').fill('browser-owned')
    await upstream.locator('select').first().selectOption('udp')
    await upstream.getByPlaceholder('例如 https://dns.google/dns-query 或 223.5.5.5').fill('127.0.0.1:15400')
    await upstream.getByRole('button', { name: '保存', exact: true }).click()
    await editor.getByRole('button', { name: '保存', exact: true }).click()
    if (role === 'go') await page.waitForTimeout(3500)
    const card = page.locator('.special-group-card').filter({ hasText: name })
    await card.waitFor()
    await card.getByRole('button', { name: '编辑', exact: true }).click()
    await page.locator('#special-group-name-vue').fill(name + '-edited')
    await page.locator('#special-group-port-vue').fill('15355')
    await editor.locator('.special-port-toggle-row').click()
    if (!await page.locator('#special-group-port-only-vue').isChecked()) throw new Error('custom-port toggle did not change')
    await editor.getByRole('button', { name: '保存', exact: true }).click()
    if (role === 'go') await page.waitForTimeout(3500)
    const edited = page.locator('.special-group-card').filter({ hasText: name + '-edited' })
    await edited.waitFor()
    const screenshot = path.join(root, `browser-${shell === '/' ? 'main' : 'log'}.png`)
    await page.screenshot({ path: screenshot, fullPage: true })
    await edited.getByRole('button', { name: '删除', exact: true }).click()
    await page.getByRole('dialog', { name: '确认框' }).getByRole('button', { name: '确认', exact: true }).click()
    await edited.waitFor({ state: 'detached' })
    if (role === 'go') await page.waitForTimeout(3500)
    if (pageErrors.length) throw new Error(JSON.stringify(pageErrors))
    results.push({ shell, create: true, edit: true, delete: true, source: 'normal', privateUpstream: true, customPortOnly: 15355, pageErrors })
    await page.close()
  }
  fs.writeFileSync(path.join(root, 'browser.json'), JSON.stringify(results, null, 2))
} finally {
  await browser.close()
}
