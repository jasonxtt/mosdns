// Real native mutation with injected discovery transport/schema failures only.
import {createRequire} from 'node:module'
import {spawn} from 'node:child_process'
import {mkdir,writeFile} from 'node:fs/promises'
import net from 'node:net'
const {chromium}=createRequire(import.meta.url)('/root/mosdns-rust-webui-20261004/browser-tools/node_modules/playwright')
function expect(locator){async function check(read,value){for(let i=0;i<100;i++){if(await read()===value)return;await new Promise(r=>setTimeout(r,50))}throw new Error('DOM retention assertion failed: '+locator)}return {toBeVisible:()=>locator.waitFor({state:'visible',timeout:5000}),toBeDisabled:()=>check(()=>locator.isDisabled(),true),toBeEnabled:()=>check(()=>locator.isDisabled(),false)}}
const root='/root/mosdns-rust-webui-20261004',label=process.argv[2],dir=`${root}/evidence/${label}`,runtime=`${dir}/runtime`
await mkdir(`${runtime}/webinfo`,{recursive:true});await mkdir(`${runtime}/rule`,{recursive:true});await mkdir(`${runtime}/cache`,{recursive:true})
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
const api=await port(),dns=await port(),group=await port(),origin=`http://127.0.0.1:${api}`
await writeFile(`${runtime}/config.yaml`,`log: {level: error}\nnative_management: {special_groups: true}\napi: {http: '127.0.0.1:${api}'}\nplugins:\n - tag: default_forward\n   type: forward\n   args: {upstreams: [{addr: 'udp://127.0.0.1:9'}]}\n - tag: main_entry\n   type: sequence\n   args: [{exec: $special_upstream_matcher}, {exec: $default_forward}]\n - tag: main\n   type: udp_server\n   args: {entry: main_entry, listen: '127.0.0.1:${dns}', enable_audit: true}\n`)
await writeFile(`${runtime}/webinfo/special_upstream_groups.json`,JSON.stringify([{slot:50,name:'retained-group',listen_port:group,custom_port_only:false}]))
await writeFile(`${runtime}/webinfo/upstream_overrides.json`,JSON.stringify({special_upstream_50:[{tag:'fixture',enabled:true,protocol:'udp',addr:'udp://127.0.0.1:9'}]}))
await writeFile(`${runtime}/rule/retained.txt`,'full:retained.example\n')
const child=spawn(process.env.NATIVE_BINARY||`${root}/evidence/mosdns-native`,['start','-c','config.yaml'],{cwd:runtime});let stderr='';child.stderr.on('data',b=>stderr+=b);const ended=new Promise(r=>child.once('exit',(code,signal)=>r({code,signal})));let browser
const result={status:'RUNNING',cases:[]}
try{
 for(let i=0;i<200;i++){try{if((await fetch(origin+'/api/v1/system/health')).ok)break}catch{}await new Promise(r=>setTimeout(r,25))}
 const caps=await (await fetch(origin+'/api/v1/capabilities')).json()
 const seed=await fetch(origin+'/plugins/special_route_50/config/retained-catalog',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({name:'retained-catalog',type:'special_50',files:'rule/retained.txt',enabled:true,url:'',auto_update:false,enable_regexp:false})});if(!seed.ok)throw new Error('seed '+seed.status+' '+await seed.text())
 browser=await chromium.launch({headless:true,args:['--no-sandbox']})
 for(const shell of ['/','/log'])for(const fault of ['500','network','invalid']){
  const page=await browser.newPage();await page.goto(origin+shell,{waitUntil:'networkidle'})
  await page.locator(shell==='/'?'nav.legacy-main-nav':'nav.log1-primary-nav').getByRole('button',{name:'规则管理',exact:true}).click()
  await page.locator(shell==='/'?'nav.legacy-sub-nav':'nav.log1-secondary-nav').getByRole('button',{name:'订阅规则',exact:true}).click()
  const row=page.locator('.rules-diversion-table tbody tr').filter({hasText:'retained-catalog'});await expect(row).toBeVisible()
  let attempts=0;await page.route('**/api/v1/capabilities',async route=>{attempts++;if(attempts>1)return route.continue();if(fault==='network')return route.abort();return route.fulfill({status:fault==='500'?500:200,contentType:'application/json',body:JSON.stringify(fault==='500'?{error:'injected'}:{...caps,ui_operations:{}})})})
  const put=page.waitForResponse(r=>new URL(r.url()).pathname==='/plugins/special_route_50/config/retained-catalog'&&r.request().method()==='PUT');put.catch(()=>{})
  await row.locator('label.switch').click();if((await put).status()!==200)throw new Error('mutation not accepted');result.current={shell,fault,initialCatalogVisible:true,accepted:200}
  await expect(page.getByRole('button',{name:'重试能力发现',exact:true})).toBeVisible()
  await expect(row).toBeVisible();await expect(row.locator('input[type=checkbox]')).toBeDisabled();await expect(row.getByRole('button',{name:'编辑',exact:true})).toBeDisabled()
  await page.screenshot({path:`${dir}/${shell==='/'?'root':'log'}-${fault}.png`,fullPage:true})
  await page.getByRole('button',{name:'重试能力发现',exact:true}).click();await expect(row.locator('input[type=checkbox]')).toBeEnabled()
  const catalog=await (await fetch(origin+'/plugins/special_route_50/config')).json(),item=catalog.find(x=>x.name==='retained-catalog');if(!item)throw new Error('catalog absent');if(await row.locator('input[type=checkbox]').isChecked()!==item.enabled)throw new Error('retry stale data')
  result.cases.push({shell,fault,accepted:200,rowsRetained:true,disabled:true,retryRefreshed:true,attempts});await page.close()
 }
 result.status='PASS'
}catch(e){result.status='FAIL';result.error=String(e);process.exitCode=1}
finally{if(browser)await browser.close();child.kill('SIGINT');result.exit=await Promise.race([ended,new Promise(r=>setTimeout(()=>r(null),5000))]);if(!result.exit){child.kill('SIGKILL');await ended;result.status='FAIL';result.error='forced shutdown';process.exitCode=1}result.stderr=stderr;await writeFile(`${dir}/result.json`,JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result))}
