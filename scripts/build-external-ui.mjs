// Builds the maintained UI as a movable external package for any `<config root>/ui/<name>/`
// (the package does not depend on the directory name; it is served at `/<name>/`):
//   webui-log/dist-external/package/{index.html, app.js, app.css, chunks/, assets/, ui-manifest.json}
// and release/mosdns-ui-<version>.zip with those files at the top level.
// Run through `npm run build:external` in webui-log. UI_VERSION overrides the version
// (default: git describe, then "dev"). Needs the `zip` command for the archive.
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { archiveName, buildManifest, validVersion } from './external-ui-manifest.mjs'

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const workspace = path.join(repository, 'webui-log')
const output = path.join(workspace, 'dist-external', 'package')

function describe() {
  try {
    return execFileSync('git', ['describe', '--tags', '--match', 'v*', '--always', '--dirty'], { cwd: repository, encoding: 'utf8' }).trim()
  } catch {
    return 'dev'
  }
}

const version = process.env.UI_VERSION || describe()
if (!validVersion(version)) throw new Error(`invalid UI version: ${JSON.stringify(version)}`)

execFileSync('npx', ['vite', 'build', '--config', 'vite.external.config.js'], {
  cwd: workspace,
  stdio: 'inherit',
  env: { ...process.env, UI_VERSION: version }
})
if (!existsSync(path.join(output, 'index.html'))) throw new Error('the external build produced no index.html')
writeFileSync(path.join(output, 'ui-manifest.json'), `${JSON.stringify(buildManifest({ version }), null, 2)}\n`)

const releaseDirectory = path.join(repository, 'release')
mkdirSync(releaseDirectory, { recursive: true })
const archive = path.join(releaseDirectory, archiveName(version))
rmSync(archive, { force: true })
execFileSync('zip', ['-r', '-X', '-q', archive, '.'], { cwd: output, stdio: 'inherit' })
console.log(`external UI ${version}: ${output}\narchive: ${archive}`)
