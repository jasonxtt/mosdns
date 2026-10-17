// The `ui-manifest.json` of an external UI package (a directory `<config root>/ui/<name>/`, any name).
// The host judges it in rust/native-host/src/ui_manifest.rs; keep API_LEVEL equal to
// that file's HOST_API_LEVEL (a Node test compares them).
export const API_LEVEL = 1

export function validVersion(version) {
  return typeof version === 'string' && version.trim() !== '' && [...version].length <= 64 && !/[\u0000-\u001f\u007f-\u009f]/.test(version)
}

export function buildManifest({ version, now = new Date() }) {
  if (!validVersion(version)) throw new Error(`invalid UI version: ${JSON.stringify(version)}`)
  return {
    manifest_schema: 1,
    ui_version: version,
    built_at: now.toISOString().replace(/\.\d+Z$/, 'Z'),
    api_level: API_LEVEL,
    requires_api_level: API_LEVEL
  }
}

/** File-system-safe form of a version for the zip name. */
export function archiveName(version) {
  return `mosdns-ui-${version.replace(/[^A-Za-z0-9._-]/g, '_')}.zip`
}
