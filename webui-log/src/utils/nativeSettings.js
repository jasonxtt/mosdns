// Helpers for the settings-backed pages of the native runtime (M1-f2):
// the proxy switch of an upstream entry and the "modified / restore default"
// marks taken from GET /api/v1/settings/effective.

// Protocols whose connection can go through a SOCKS5 proxy; UDP never does.
const PROXY_PROTOCOLS = new Set(['tcp', 'tls', 'dot', 'https', 'doh'])

// The native runtime carries the proxy switch of an entry when it does not
// list `socks_proxy` among its unsupported features.
export function settingsBackedProxy(capabilities) {
  return capabilities?.kind === 'native' &&
    Array.isArray(capabilities.unsupported_features) &&
    !capabilities.unsupported_features.includes('socks_proxy')
}

export function protocolUsesProxy(protocol) {
  return PROXY_PROTOCOLS.has(String(protocol || '').trim().toLowerCase())
}

// The `use_socks_proxy` the editor sends for an entry: only protocols that can
// use a proxy carry a checked switch.
export function nativeProxyFlag(protocol, checked) {
  return protocolUsesProxy(protocol) ? Boolean(checked) : false
}

// Where each settable item comes from: `{ socks5, ecs, upstreams: {group: source} }`
// with `''` for an item the configuration does not have. Several proxy or ECS
// objects make the page ambiguous (the backend answers 409), so the first wins.
export function settingsSources(effective) {
  const result = { socks5: '', ecs: '', upstreams: {} }
  const items = Array.isArray(effective?.items) ? effective.items : []
  for (const item of items) {
    if (!item || typeof item !== 'object') continue
    if (item.kind === 'socks5_proxy' && !result.socks5) result.socks5 = String(item.source || '')
    else if (item.kind === 'ecs_address' && !result.ecs) result.ecs = String(item.source || '')
    else if (item.kind === 'aliapi') result.upstreams[String(item.object)] = String(item.source || '')
  }
  return result
}

export function isModified(source) {
  return source === 'settings'
}

export const NATIVE_OVERRIDES_NOTE =
  '保存后立即生效，无需重启。留空或点“恢复默认”回到配置文件（YAML）里的值；默认配置包中代理为空表示不使用代理。'

export const NATIVE_REPLACEMENTS_NOTE =
  '原生运行时不支持任意字符串替换规则。代理与 ECS 请用上方覆盖设置；其它改动请直接修改 YAML 配置。'
