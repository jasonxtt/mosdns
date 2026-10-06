// Build-only Node helper. The resulting native executable does not use Node.
import { createHash } from 'node:crypto'
import { lstatSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
const root = process.cwd()
const version = process.env.MOSDNS_BUILD_VERSION
if (!version?.trim() || /[\r\n]/.test(version)) throw new Error('invalid product version')
const [mode, manifestPath, artifactPath] = process.argv.slice(2)
const hash = p => createHash('sha256').update(readFileSync(path.resolve(root,p))).digest('hex')
if (mode === 'stamp') {
  console.log(encodeURIComponent(version))
} else if (mode === 'prepare') {
  const files = new Map()
  function collect(relative, embedded = false) {
    const stat = lstatSync(path.join(root,relative))
    if (stat.isSymbolicLink()) throw new Error(`symlink build input: ${relative}`)
    if (stat.isDirectory()) {
      for (const name of readdirSync(path.join(root,relative)).sort()) {
        if (['target','node_modules','.git'].includes(name)) continue
        collect(`${relative}/${name}`,embedded)
      }
    } else if (stat.isFile()) {
      const name=path.basename(relative)
      if (embedded && (name.startsWith('.') || /\.(map|pem|key)$/.test(name))) throw new Error(`unsafe embedded file: ${relative}`)
      files.set(relative,hash(relative))
    } else throw new Error(`non-regular build input: ${relative}`)
  }
  for (const dir of ['rust','webui-log/src','webui-log/src-log1']) collect(dir)
  for (const file of ['webui-log/package.json','webui-log/package-lock.json','webui-log/vite.config.js','webui-log/vite.log1.config.js','webui-log/index.html','webui-log/log1.index.html','scripts/build-rust-native.sh','scripts/native-build-manifest.mjs']) collect(file)
  collect('coremain/www/assets',true)
  for (const file of ['log.html','log1.html']) {
    collect(`coremain/www/${file}`,true)
    const html=readFileSync(`coremain/www/${file}`,'utf8')
    const refs=[...html.matchAll(/["'](\/assets\/[^"']+)["']/g)].map(m=>m[1])
    if (!refs.some(ref=>ref.includes('/app.js?')) || !refs.some(ref=>ref.includes('/app.css?'))) throw new Error(`missing UI roots: ${file}`)
    for (const ref of refs) {
      const url=new URL(ref,'http://build.invalid')
      if (!files.has(`coremain/www${url.pathname}`)) throw new Error(`missing root asset: ${ref}`)
      if (/\/app\.(js|css)$/.test(url.pathname) && url.searchParams.get('v')!==version) throw new Error(`version stamp mismatch: ${ref}`)
    }
  }
  const source=Object.fromEntries([...files].sort(([a],[b])=>a.localeCompare(b)))
  const assets=Object.fromEntries(Object.entries(source).filter(([p])=>p.startsWith('coremain/www/')))
  writeFileSync(manifestPath,JSON.stringify({schema_version:1,runtime:'rust',version,source_id:process.env.BUILD_SOURCE_ID||null,source,assets,source_manifest_sha256:createHash('sha256').update(JSON.stringify(source)).digest('hex'),artifact:null},null,2)+'\n')
} else if (mode === 'finish') {
  const manifest=JSON.parse(readFileSync(manifestPath,'utf8'))
  for (const [file,digest] of Object.entries(manifest.source)) if (hash(file)!==digest) throw new Error(`input changed during build: ${file}`)
  const actual=execFileSync(artifactPath,['version'],{encoding:'utf8'})
  if (actual!==`${version}\n` || manifest.version!==version) throw new Error('native version mismatch')
  manifest.artifact={path:artifactPath,sha256:hash(artifactPath),version_stdout:actual}
  manifest.rustc=execFileSync('rustc',['-vV'],{encoding:'utf8'}).trim()
  writeFileSync(manifestPath,JSON.stringify(manifest,null,2)+'\n')
} else throw new Error('expected stamp, prepare or finish')
