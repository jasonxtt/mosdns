import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { validateRuntimeCapabilities, createRuntimeCapabilityClient } from '../src/api/runtimeCapabilities.js'
const old = {schema_version:1, runtime:'rust',special_groups:{enabled:false},upstream_protocols:[],rule_formats:[],unsupported_features:[],endpoints:{audit_v1:true,audit_v2:true,cache_inventory_get:true,metrics_get:true,manual_rules:{show_get:true,save_get:true,post:false},special_groups:{get:true,post:false,delete:false},upstream:{tags_get:true,config_get:true,runtime_get:true,config_post:false},diversion_sources:{list_get:false,put:false,delete:false}}}
test('present incomplete operation matrix is invalid discovery, never legacy',()=>{
 assert.throws(()=>validateRuntimeCapabilities({...old,ui_operations:{}}),/invalid runtime/)
})
test('old Rust schema1 uses explicit conservative operations, not Go',async()=>{
 const value=await createRuntimeCapabilityClient(async()=>old)()
 assert.equal(value.kind,'native')
 assert.equal(value.ui_operations['audit.read'].supported,true)
 assert.equal(value.ui_operations['rules.local.manage'].supported,false)
 assert.equal(value.ui_operations['capture.logs'].supported,false)
 assert.equal(value.ui_operations['system.health'].supported,false)
})

const runtime = await import('../src/api/runtimeCapabilities.js')
const {OPERATION_IDS,capabilityState,capabilityFetch,requestOperation,refreshRuntimeCapabilities,supportsOperation,switchTagForType,switchValueFromResponse} = runtime
const matrix = (supported = []) => Object.fromEntries(OPERATION_IDS.map(id=>[id,{supported:supported.includes(id),reason:supported.includes(id)?null:'未支持'}]))
const native = (supported = []) => ({...old,ui_operations:matrix(supported)})
const install = value => {capabilityState.status='ready';capabilityState.value={kind:'native',...value};capabilityState.error='';capabilityState.cacheTags=new Set()}
test('every false family suppresses actual fetch for reads, refreshes and stale writes',async()=>{
 install(native())
 let calls=0;const original=globalThis.fetch;globalThis.fetch=async()=>{calls++;throw new Error('network reached')}
 const paths=[
 ['system.health','/api/v1/system/health'],['audit.read','/api/v2/audit/logs'],['audit.control','/api/v1/audit/start'],['audit.control','/api/v1/audit/stop'],['audit.control','/api/v1/audit/clear'],['audit.capacity','/api/v1/audit/capacity'],['query.rank','/api/v2/audit/rank/domain'],['cache.inventory','/api/v1/cache/inventory'],['cache.manage','/plugins/cache_one/flush'],['metrics.cache','/metrics'],['rules.local.read','/plugins/special_manual_50/show'],['rules.local.manage','/plugins/special_manual_50/post'],['groups.read','/api/v1/special-groups'],['groups.manage','/api/v1/special-groups/50','DELETE'],['upstreams.read','/api/v1/upstream/tags'],['upstreams.manage','/api/v1/upstream/config','POST'],['rules.diversion','/plugins/diversion_50/config'],['rules.adguard','/plugins/adguard/rules'],['rules.adguard','/plugins/adguard/update/1','POST'],['capture.logs','/api/v1/capture/start','POST'],['capture.logs','/api/v1/capture/logs'],['client.aliases','/plugins/clientname'],['switches.manage','/plugins/switch3/show'],['switches.manage','/plugins/switch17/post','POST'],['cache.requery','/plugins/requery/trigger','POST'],['cache.requery','/plugins/requery/scheduler/config','POST'],['lists.remembered','/plugins/my_fakeiplist/show'],['lists.remembered','/plugins/top_domains/save'],['appearance.server','/api/v1/appearance/panel-background'],['appearance.server','/api/v1/appearance/panel-background/upload','POST'],['appearance.server','/api/v1/appearance/panel-background/history/remove','DELETE'],['system.restart','/api/v1/system/restart','POST'],['system.webui_port','/api/v1/system/webui-port'],['system.config_management','/api/v1/config/export','POST'],['system.update','/api/v1/update/status'],['system.domain_generation','/api/v1/domain-generation'],['system.global_overrides','/api/v1/overrides']]
 try{for(const [id,url,method='GET'] of paths){assert.equal(requestOperation(url,method),id,url);await assert.rejects(()=>capabilityFetch(url,{method}),e=>e.capabilityDisabled===true)}}finally{globalThis.fetch=original}
 assert.equal(calls,0)
})
test('new matrix requires all entries and valid bool/reason, allows additive unknown keys',()=>{
 for(const id of OPERATION_IDS){const value=native();delete value.ui_operations[id];assert.throws(()=>validateRuntimeCapabilities(value),/invalid runtime/)}
 for(const entry of [{supported:'true',reason:null},{supported:true,reason:'x'},{supported:false,reason:null},{supported:false,reason:' '}])assert.throws(()=>validateRuntimeCapabilities({...old,ui_operations:{...matrix(), 'system.health':entry}}),/invalid runtime/)
 assert.equal(validateRuntimeCapabilities({...native(),ui_operations:{...matrix(),future:{}}}).runtime,'rust')
})
test('exact old native fallback maps each strict endpoint boolean and never health/process/Go families',()=>{
 const managed={...old,special_groups:{enabled:true},endpoints:{...old.endpoints,manual_rules:{show_get:true,post:true,save_get:true},special_groups:{get:true,post:true,delete:true},upstream:{tags_get:true,config_get:true,runtime_get:true,config_post:true},diversion_sources:{list_get:true,put:true,delete:true}}}
 const value=validateRuntimeCapabilities(managed)
 const yes=['audit.read','audit.control','audit.capacity','query.rank','cache.inventory','cache.manage','metrics.cache','rules.local.read','rules.local.manage','groups.read','groups.manage','upstreams.read','upstreams.manage','rules.diversion']
 for(const id of OPERATION_IDS){assert.equal(value.ui_operations[id].supported,yes.includes(id),id);assert.equal(value.ui_operations[id].reason===null,yes.includes(id),id)}
 const unverified=validateRuntimeCapabilities({...old,endpoints:{audit_v1:'true',manual_rules:{show_get:1,post:true,save_get:false}}})
 for(const id of OPERATION_IDS)assert.equal(unverified.ui_operations[id].supported,false,id)
})
test('discovery errors remain retryable without optional network requests',async()=>{
 let attempts=0;const client=createRuntimeCapabilityClient(async()=>{attempts++;if(attempts===1)throw Object.assign(new Error('unauthorized'),{status:401});return old})
 await assert.rejects(client(),/unauthorized/);assert.equal((await client()).kind,'native');assert.equal(attempts,2)
 install(native());const original=globalThis.fetch;const calls=[]
 globalThis.fetch=async url=>{calls.push(url);if(calls.length===1)throw new Error('offline');return Response.json(native(['audit.read']))}
 try{
  await assert.rejects(refreshRuntimeCapabilities(),/offline/);assert.equal(capabilityState.status,'error')
  await assert.rejects(()=>capabilityFetch('/plugins/switch17/show'),/offline/);assert.equal(calls.length,1)
  await refreshRuntimeCapabilities();assert.equal(supportsOperation('audit.read'),true);assert.equal(calls.length,2)
 }finally{globalThis.fetch=original}
})
test('successful inventory mutations reload capabilities and named cache tags retain own gate',async()=>{
 install(native(['groups.manage','cache.inventory','cache.manage']))
 const original=globalThis.fetch;const calls=[]
 globalThis.fetch=async url=>{calls.push(url);if(url==='/api/v1/capabilities')return Response.json(native(['groups.manage','rules.local.manage','cache.inventory','cache.manage']));if(url==='/api/v1/cache/inventory')return Response.json({schema_version:1,caches:[{tag:'hot_answers'}]});return Response.json({ok:true})}
 try{
  await capabilityFetch('/api/v1/special-groups',{method:'POST',body:'{}'});assert.equal(supportsOperation('rules.local.manage'),true);assert.deepEqual(calls.slice(0,2),['/api/v1/special-groups','/api/v1/capabilities'])
  await capabilityFetch('/api/v1/cache/inventory');assert.equal(requestOperation('/plugins/hot_answers/save'),'cache.manage');await capabilityFetch('/plugins/hot_answers/save')
 }finally{globalThis.fetch=original}
})
test('unadvertised audit version and separately disabled switch followup never reach fetch',async()=>{
 install({...native(['audit.read','switches.manage']),endpoints:{...old.endpoints,audit_v2:false}})
 let calls=0;const original=globalThis.fetch;globalThis.fetch=async()=>{calls++;return Response.json({})}
 try{await assert.rejects(()=>capabilityFetch('/api/v2/audit/logs'),e=>e.capabilityDisabled);await assert.rejects(()=>capabilityFetch('/plugins/requery/trigger',{method:'POST'}),e=>e.capabilityDisabled);await assert.rejects(()=>capabilityFetch('/plugins/switch17/show'),e=>e.capabilityDisabled);await capabilityFetch('/api/v1/audit/logs');assert.equal(calls,1)}finally{globalThis.fetch=original}
})
test('native switch requests resolve configured tags and carry the discovered generation',async()=>{
 install({...native(['switches.manage']),switches:{schema_version:1,config_generation:'7',instances:[{type:'switch17',tag:'routing/custom',readable:true,writable:true,reason:null}]}})
 assert.equal(switchTagForType(17), 'routing/custom')
 assert.equal(requestOperation('/plugins/routing%2Fcustom/show'), 'switches.manage')
 assert.equal(requestOperation('/plugins/switch17/show'), 'switches.manage')
 const original=globalThis.fetch;const calls=[]
 globalThis.fetch=async(url,options)=>{calls.push({url,options});return new Response('A')}
 try{await capabilityFetch('/plugins/routing%2Fcustom/post',{method:'POST',headers:{'Content-Type':'application/json'},body:'{"value":"A"}'})
  assert.equal(calls[0].options.headers.get('X-Mosdns-Config-Generation'),'7')
 }finally{globalThis.fetch=original}
})
test('native switch inventory rejects malformed entries and old native schemas stay switch-disabled',async()=>{
 assert.throws(()=>validateRuntimeCapabilities({...old,switches:{schema_version:1,config_generation:'0',instances:[{type:'switch3',tag:'x',readable:true,writable:false,reason:null}]}}),/switch capability/)
 const value=await createRuntimeCapabilityClient(async()=>old)()
 assert.equal(value.ui_operations['switches.manage'].supported,false)
})
test('both system shells preserve exact native switch whitespace on reload', async()=>{
 const raw=' A \n'
 assert.equal(switchValueFromResponse(raw,'native'),raw)
 assert.equal(switchValueFromResponse(raw,'legacy'),'A')
 for (const path of [
  new URL('../src/components/SystemControlManager.vue', import.meta.url),
  new URL('../src-log1/SystemControlManager.vue', import.meta.url)
 ]) {
  const source=await readFile(path,'utf8')
  assert.match(source,/switchValueFromResponse\(item\.value, capabilityState\.value\?\.kind\)/)
 }
})
test('unmanaged native local menu retains only actual eligible standard providers and propagates IO failures',async()=>{
 const {loadNativeLocalProfiles}=await import('../src/api/nativeManagement.js')
 const profiles=[{tag:'whitelist'},{tag:'blocklist'},{tag:'direct_ip'}];const calls=[]
 const eligible=await loadNativeLocalProfiles(profiles,async url=>{calls.push(url);if(url.includes('whitelist'))throw Object.assign(new Error('missing'),{status:404});if(url.includes('direct_ip'))throw Object.assign(new Error('not a managed domain provider'),{status:400});return 'a.example'})
 assert.deepEqual(eligible,[profiles[1]]);assert.equal(calls.length,3)
 await assert.rejects(()=>loadNativeLocalProfiles(profiles,async()=>{throw Object.assign(new Error('disk failure'),{status:500})}),/disk failure/)
})
test('accepted old schema1 managed/unmanaged fixtures use the exact approved per-operation fallback',async()=>{
 const {readFile}=await import('node:fs/promises')
 for(const managed of [true,false]){
  const fixture=JSON.parse(await readFile(new URL(`../../docs/rust/validation-records/10-04-rust-native-webui-runtime-capabilities/s5-old-native-${managed?'managed':'unmanaged'}.json`,import.meta.url),'utf8'))
  assert.equal(Object.hasOwn(fixture,'ui_operations'),false)
  const actual=validateRuntimeCapabilities(fixture).ui_operations
  for(const id of OPERATION_IDS){
   const always=['audit.read','audit.control','audit.capacity','query.rank','cache.inventory','cache.manage','metrics.cache','rules.local.read','groups.read','upstreams.read']
   const gated=['rules.local.manage','groups.manage','upstreams.manage','rules.diversion']
   assert.equal(actual[id].supported,always.includes(id)||(managed&&gated.includes(id)),`${managed}:${id}`)
  }
 }
})
test('accepted mutation ACK survives discovery refresh failure and existing view stays mounted but disabled',async()=>{
 install(native(['groups.manage']))
 const original=globalThis.fetch
 globalThis.fetch=async url=>{if(url==='/api/v1/capabilities')throw new Error('discovery temporarily failed');return Response.json({accepted:true})}
 try{
  const response=await capabilityFetch('/api/v1/special-groups',{method:'POST',body:'{}'})
  assert.equal(response.ok,true);assert.equal((await response.json()).accepted,true)
  assert.equal(capabilityState.status,'error');assert.equal(capabilityState.value.kind,'native');assert.equal(supportsOperation('groups.manage'),false)
  await assert.rejects(()=>capabilityFetch('/api/v1/special-groups',{method:'POST'}),/discovery temporarily failed/)
 }finally{globalThis.fetch=original}
})
test('unknown additive kind cannot override internal native classification',async()=>{
 const value=await createRuntimeCapabilityClient(async()=>({...native(),kind:'legacy'}))()
 assert.equal(value.kind,'native');assert.equal(value.ui_operations['system.restart'].supported,false)
})
test('native inventory failures never invent a Go cache menu',async()=>{
 const {loadCacheInventory}=await import('../src/utils/cacheInventory.js')
 await assert.rejects(()=>loadCacheInventory(async()=>{throw Object.assign(new Error('404'),{status:404})},{native:true}),/404/)
 await assert.rejects(()=>loadCacheInventory(async()=>({supported:false}),{native:true}),/不一致/)
 assert.equal(await loadCacheInventory(async()=>{throw Object.assign(new Error('404'),{status:404})}),null)
})
