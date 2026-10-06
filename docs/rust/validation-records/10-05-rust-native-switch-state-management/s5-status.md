# S5 validation — truthful controls in both Vue shells

Status: implementation green; cumulative C2C delta review PASS.

The shared runtime capability client validates the optional switch inventory,
canonical generation, unique type/tag entries, and read/write reasons. Native
switch requests resolve configured tags before local-rule/cache fallback and
carry `X-Mosdns-Config-Generation`; missing native inventory fails closed.
The main and compatibility `SystemControlManager` shells render the generic
inventory panel, preserve legacy Go presets, and resolve type 3/type 17 reads
through the discovered inventory. Native views do not claim product-specific
cache/requery effects.

Observed results:

```text
node --test webui-log/tests/*.test.mjs
29 passed, 0 failed
npm run build
success
npm run build:log1
success
```

The tests include native custom-tag request admission/generation headers,
malformed inventory rejection, old-native schema fail-closed behavior, absent
inventory zero-request behavior, and legacy 404 fallback. The two Vite builds
produce the embedded `/` and `/log` assets consumed by the native build. The
final post-drain delta review returned `FINAL: PASS` on the exact 9-path
delta; see the S4 review record for its C2C conversation and snapshot identity.
