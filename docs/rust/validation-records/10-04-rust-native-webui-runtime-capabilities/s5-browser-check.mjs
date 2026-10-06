import { createRequire } from 'node:module'
import { spawn, execFileSync } from 'node:child_process'
import { mkdir, writeFile, readFile } from 'node:fs/promises'
import net from 'node:net'
const require=createRequire(import.meta.url)
const {chromium}=require('/root/mosdns-rust-webui-20261004/browser-tools/node_modules/playwright')
const root='/root/mosdns-rust-webui-20261004'
const label=process.argv[2]||'s5-baseline'
const dir=`${root}/evidence/${label}`
await mkdir(dir,{recursive:true})
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
const api=await port(),dns=await port()
const runtime=`${dir}/runtime`;await mkdir(runtime,{recursive:true})
await writeFile(`${runtime}/config.yaml`,`log: {level: error}\nnative_management: {special_groups: true}\napi: {http: '127.0.0.1:${api}'}\nplugins:\n  - tag: default_forward\n    type: forward\n    args: {upstreams: [{addr: 'udp://127.0.0.1:9'}]}\n  - tag: main_entry\n    type: sequence\n    args: [{exec: $special_upstream_matcher}, {exec: $default_forward}]\n  - tag: main\n    type: udp_server\n    args: {entry: main_entry, listen: '127.0.0.1:${dns}', enable_audit: true}\n`)
const binary=process.env.NATIVE_BINARY||`${root}/source/rust/target/debug/mosdns`
const expectedVersion=execFileSync(binary,['version'],{encoding:'utf8'}).trim()
const child=spawn(binary,['start','-c',`${runtime}/config.yaml`],{cwd:runtime})
let stderr='';child.stderr.on('data',b=>stderr+=b)
const exited=new Promise(r=>child.once('exit',(code,signal)=>r({code,signal})))
const result={api,dns,pid:child.pid,pages:[],unsupported:[],errors:[]}
let browser
try{
 for(let i=0;i<100;i++){try{if((await fetch(`http://127.0.0.1:${api}/api/v1/system/health`)).ok)break}catch{}await new Promise(r=>setTimeout(r,50))}
 const capabilities=await (await fetch(`http://127.0.0.1:${api}/api/v1/capabilities`)).json();result.capabilities=capabilities;const oldCapabilities={...capabilities};delete oldCapabilities.ui_operations;await writeFile(`${dir}/old-native-managed.json`,JSON.stringify(oldCapabilities,null,2)+'\n')
 browser=await chromium.launch({headless:true,args:['--no-sandbox']})
 for(const path of ['/','/log']){
  const page=await browser.newPage();const requests=[]
  page.on('request',r=>{const requested=new URL(r.url()).pathname;requests.push(requested);if(/^(\/api\/v1\/(appearance|capture|update|domain-generation|overrides|config|system\/(restart|webui-port))|\/plugins\/(clientname|switch\d+|requery|adguard|my_|top_domains))/.test(requested))result.unsupported.push({shell:path,url:r.url()})})
  page.on('pageerror',e=>result.errors.push({path,message:e.message}))
  await page.goto(`http://127.0.0.1:${api}${path}`,{waitUntil:'networkidle'})
  const visits=[]
  if(label!=='s5-baseline') {
   const mainSelector=path==='/'?'nav.legacy-main-nav > button':'nav.log1-primary-nav button'
   const main=page.locator(mainSelector)
   const count=await main.count();if(count!==(path==='/'?6:5))throw new Error(`main tab coverage ${count}`)
   for(let i=0;i<count;i++) {
    await main.nth(i).click();await page.waitForTimeout(250)
    const secondary=page.locator(path==='/'?'nav.legacy-sub-nav button':'nav.log1-secondary-nav button')
    const n=await secondary.count()
    if(n===0)visits.push({main:i,text:await main.nth(i).innerText()})
    for(let j=0;j<n;j++) {await secondary.nth(j).click();await page.waitForTimeout(200);await page.evaluate(()=>window.dispatchEvent(new CustomEvent('mosdns-log-refresh')));await page.waitForTimeout(200);visits.push({main:i,secondary:j,text:await secondary.nth(j).innerText()})}
    await page.evaluate(()=>window.dispatchEvent(new CustomEvent('mosdns-log-refresh')));await page.waitForTimeout(250)
   }
  }
  if(label!=='s5-baseline' && visits.length < 9)throw new Error(`insufficient tab coverage ${path}: ${visits.length}`)
  result.pages.push({path,title:await page.title(),requests,visits,boundaries:await page.locator('[data-operation]').evaluateAll(nodes=>nodes.map(n=>({operation:n.dataset.operation,disabled:n.disabled,reason:n.querySelector('[role=note]')?.textContent||''})))})
  await page.screenshot({path:`${dir}/${path==='/'?'root':'log'}.png`,fullPage:true})
  await page.close()
 }
 if(label!=='s5-baseline') {
  result.discovery=[]
  for(const path of ['/','/log']) {
   // Initial discovery remains visible and emits zero optional requests.
   const pendingPage=await browser.newPage();const seen=[];let release
   pendingPage.on('request',request=>{const p=new URL(request.url()).pathname;if(p.startsWith('/api/')||p.startsWith('/plugins/')||p==='/metrics')seen.push(p)})
   await pendingPage.route('**/api/v1/capabilities',async route=>{await new Promise(resolve=>release=resolve);await route.continue()})
   await pendingPage.goto(`http://127.0.0.1:${api}${path}`,{waitUntil:'domcontentloaded'})
   await pendingPage.getByRole('alert').filter({hasText:'正在读取运行时能力'}).waitFor()
   if(seen.some(p=>p!=='/api/v1/capabilities'))throw new Error(`pending optional request ${path}`)
   release();await pendingPage.locator('main').waitFor();await pendingPage.close()
   for(const fault of ['non404','invalid']) {
    const page=await browser.newPage();const seen=[];let attempts=0
    page.on('request',request=>{const p=new URL(request.url()).pathname;if(p.startsWith('/api/')||p.startsWith('/plugins/')||p==='/metrics')seen.push(p)})
    await page.route('**/api/v1/capabilities',async route=>{attempts++;if(attempts===1)await route.fulfill({status:fault==='non404'?500:200,contentType:'application/json',body:JSON.stringify(fault==='non404'?{error:'injected'}:{...capabilities,ui_operations:{}})});else await route.continue()})
    await page.goto(`http://127.0.0.1:${api}${path}`,{waitUntil:'networkidle'})
    if(seen.some(p=>p!=='/api/v1/capabilities'))throw new Error(`error optional request ${path}`)
    await page.getByRole('button',{name:'重试能力发现'}).click();await page.locator('main').waitFor();await page.waitForTimeout(300)
    if(attempts!==2)throw new Error(`retry count ${attempts}`)
    if(!(await page.locator('body').innerText()).includes(`Rust · ${expectedVersion}`))throw new Error(`retry missing identity ${path}`)
    result.discovery.push({path,fault,attempts,optionalBeforeRetry:0,healthAfterRetry:seen.includes('/api/v1/system/health')})
    await page.close()
   }
   const oldPage=await browser.newPage();const oldSeen=[]
   oldPage.on('request',request=>oldSeen.push(new URL(request.url()).pathname))
   await oldPage.route('**/api/v1/capabilities',route=>route.fulfill({status:200,contentType:'application/json',body:JSON.stringify(oldCapabilities)}))
   await oldPage.goto(`http://127.0.0.1:${api}${path}`,{waitUntil:'networkidle'})
   if(oldSeen.includes('/api/v1/system/health'))throw new Error(`old Rust probed unadvertised health ${path}`)
   if(!(await oldPage.locator('body').innerText()).includes('旧原生后端未声明此能力'))throw new Error(`old schema missing native reason ${path}`)
   result.discovery.push({path,fault:'old-native-schema1',healthRequests:0})
   await oldPage.close()
   const readOnlyPage=await browser.newPage()
   if(label.startsWith('s5-reasons-red'))await readOnlyPage.route('**/assets/vue-log*/app.js*',async route=>route.fulfill({status:200,contentType:'application/javascript',body:await readFile(`${root}/evidence/s5-old-${route.request().url().includes('/vue-log1/')?'log':'root'}.js`,'utf8')}))
   const unmanaged=JSON.parse(await readFile(`${root}/source/docs/rust/validation-records/10-04-rust-native-webui-runtime-capabilities/s5-old-native-unmanaged.json`,'utf8'))
   await readOnlyPage.route('**/api/v1/capabilities',route=>route.fulfill({status:200,contentType:'application/json',body:JSON.stringify(unmanaged)}))
   await readOnlyPage.route('**/plugins/*/show*',route=>route.fulfill(new URL(route.request().url()).pathname==='/plugins/whitelist/show'?{status:200,contentType:'text/plain',body:'full:read-only.example\n'}:{status:404,body:'unavailable provider'}))
   await readOnlyPage.goto(`http://127.0.0.1:${api}${path}`,{waitUntil:'networkidle'})
   const mainNav=readOnlyPage.locator(path==='/'?'nav.legacy-main-nav':'nav.log1-primary-nav')
   if(path==='/')await mainNav.getByRole('button',{name:'规则管理',exact:true}).click()
   else {await mainNav.getByRole('button',{name:'规则管理',exact:true}).click();await readOnlyPage.locator('nav.log1-secondary-nav').getByRole('button',{name:'本地规则',exact:true}).click()}
   await readOnlyPage.locator('.list-editor').waitFor()
   await readOnlyPage.waitForFunction(()=>document.querySelector('.list-editor')?.value.includes('read-only.example'),null,{timeout:5000})
   if(await readOnlyPage.getByRole('button',{name:'保存全部改动',exact:true}).isEnabled())throw new Error(`local write enabled ${path}`)
   await readOnlyPage.locator('[data-operation-reason="rules.local.manage"]').filter({hasText:'旧原生后端未声明此能力'}).waitFor({timeout:2000})
   if(path==='/')await mainNav.getByRole('button',{name:'上游设置',exact:true}).click()
   else {await mainNav.getByRole('button',{name:'系统设置',exact:true}).click();await readOnlyPage.locator('nav.log1-secondary-nav').getByRole('button',{name:'上游设置',exact:true}).click()}
   await readOnlyPage.getByRole('button',{name:'添加上游DNS',exact:true}).waitFor()
   if(await readOnlyPage.getByRole('button',{name:'添加上游DNS',exact:true}).isEnabled())throw new Error(`upstream write enabled ${path}`)
   await readOnlyPage.locator('[data-operation-reason="upstreams.manage"]').filter({hasText:'旧原生后端未声明此能力'}).waitFor({timeout:2000})
   await readOnlyPage.getByRole('button',{name:'管理',exact:true}).click()
   if(await readOnlyPage.getByRole('button',{name:'新增专属分流组',exact:true}).isEnabled())throw new Error(`group write enabled ${path}`)
   await readOnlyPage.locator('[data-operation-reason="groups.manage"]').filter({hasText:'旧原生后端未声明此能力'}).waitFor({timeout:2000})
   result.discovery.push({path,fault:'old-native-unmanaged-read-only',localDataVisible:true,disabledWrites:['rules.local.manage','upstreams.manage','groups.manage'],adjacentReasons:true})
   await readOnlyPage.close()
  }
 }
 result.status=result.unsupported.length===0&&result.errors.length===0?'PASS':'FAIL'
}catch(error){result.status='FAIL';result.error=String(error)}
finally{
 if(browser)await browser.close()
 child.kill('SIGINT')
 const exit=await Promise.race([exited,new Promise(r=>setTimeout(()=>r(null),5000))]);if(!exit){child.kill('SIGKILL');result.cleanup={forced:true,exit:await exited};result.status='FAIL'}else result.cleanup={forced:false,exit}
 result.stderr=stderr
 await writeFile(`${dir}/result.json`,JSON.stringify(result,null,2)+'\n')
 console.log(JSON.stringify(result,null,2));process.exitCode=result.status==='PASS'?0:1
}
