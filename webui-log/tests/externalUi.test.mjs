import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { API_LEVEL, archiveName, buildManifest, validVersion } from '../../scripts/external-ui-manifest.mjs'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

test('the manifest of an external package has the fields the host judges', () => {
  const manifest = buildManifest({ version: 'v1.2.3-4-gabc', now: new Date('2026-10-17T10:11:12.345Z') })
  assert.deepEqual(manifest, {
    manifest_schema: 1,
    ui_version: 'v1.2.3-4-gabc',
    built_at: '2026-10-17T10:11:12Z',
    api_level: API_LEVEL,
    requires_api_level: API_LEVEL
  })
  for (const bad of ['', '  ', 'a\nb', 'x'.repeat(65), undefined, 5]) {
    assert.equal(validVersion(bad), false, String(bad))
    assert.throws(() => buildManifest({ version: bad }))
  }
  assert.equal(archiveName('v1.2/3 beta'), 'mosdns-ui-v1.2_3_beta.zip')
})

test('the build script and the host agree on the interface level', () => {
  const host = read('../../rust/native-host/src/ui_manifest.rs')
  assert.equal(Number(host.match(/pub\(crate\) const HOST_API_LEVEL: u64 = (\d+);/)[1]), API_LEVEL)
})

test('the external build uses relative resource URLs and a name-independent output directory', () => {
  const config = read('../vite.external.config.js')
  assert.match(config, /base: '\.\/'/)
  assert.match(config, /dist-external\/package/)
  assert.doesNotMatch(config, /\/main\//, 'nothing in the package depends on a mount name')
  assert.match(config, /index\.html/)
  const pkg = JSON.parse(read('../package.json'))
  assert.equal(pkg.scripts['build:external'], 'node ../scripts/build-external-ui.mjs')
  assert.equal(pkg.scripts.build, 'vite build', 'the built-in build is unchanged')
  const script = read('../../scripts/build-external-ui.mjs')
  assert.match(script, /vite\.external\.config\.js/)
  assert.match(script, /ui-manifest\.json/)
})

test('the built-in UI links to every mounted mosdns UI package and explains refused ones', () => {
  const panel = read('../src/components/system/SystemInfoPanel.vue')
  assert.match(panel, /capabilityState\.health\?\.external_ui/)
  assert.match(panel, /entry\.kind === 'mosdns_ui' && entry\.mounted/)
  assert.match(panel, /entry\.kind === 'mosdns_ui' && !entry\.mounted/)
  assert.match(panel, /v-for="entry in mountedPackages"/)
  assert.match(panel, /encodeURIComponent\(entry\.name\)/)
  assert.doesNotMatch(panel, /'\/main\/'|"\/main\/"/, 'no mount name is hard-coded')
  assert.match(panel, /new URL\('\.\/', window\.location\.href\)\.pathname/, 'a package does not link to itself')
})

test('a port change made in an external UI stays under that UI\'s own path', () => {
  const control = read('../src/components/SystemControlManager.vue')
  assert.match(control, /url\.pathname = new URL\("\.\/", window\.location\.href\)\.pathname;/)
  assert.doesNotMatch(control, /\/main\//)
})

test('the native build embeds the maintained UI from rust/native-host/ui and carries no compatibility UI', () => {
  const build = read('../../scripts/build-rust-native.sh')
  assert.doesNotMatch(build, /log1/)
  const manifest = read('../../scripts/native-build-manifest.mjs')
  assert.doesNotMatch(manifest, /log1|coremain/)
  assert.match(manifest, /rust\/native-host\/ui\/index\.html|'index\.html'/)
  const buildRs = read('../../rust/native-host/build.rs')
  assert.doesNotMatch(buildRs, /log1|coremain/)
  assert.match(buildRs, /join\("ui"\)/)
  const vite = read('../vite.config.js')
  assert.match(vite, /rust\/native-host\/ui\/assets\/vue-log/)
  assert.match(vite, /rust\/native-host\/ui\/index\.html/)
  assert.doesNotMatch(read('../package.json'), /log1/)
})
