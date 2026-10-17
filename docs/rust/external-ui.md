# Updating the WebUI without updating the program (external UI package)

The Rust-native program has its maintained WebUI built in at `/`. You can ship a newer UI separately and open it at `/<name>/`,
without recompiling or replacing the program; the built-in UI stays available as the fallback. `<name>` is simply the name of the
directory you put the package in: `<config root>/ui/pg/` is served at `/pg/`, `ui/a/` at `/a/` (the examples below use `pg`).
Contract: [native-webui-hosting.md](contracts/native-webui-hosting.md) ("External UI packages").

**Directory names:** non-empty, no `/` or `\`, no control characters, a real directory (not a symlink), and not one of the reserved names
`root`, `log`, `log1`, `legacy`, `blog`, `assets`, `debug`, `metrics`, `plugins`, `api`. Anything else is listed as unusable in health and is not served.

## Build a package (developer)
```sh
cd webui-log
UI_VERSION=v0.7.3-ui2 npm run build:external      # UI_VERSION is optional (default: git describe)
```
Output: `webui-log/dist-external/package/` (`index.html`, `app.js`, `app.css`, `ui-manifest.json`, …) and `release/mosdns-ui-<version>.zip`
(the same files at the zip's top level). Resource URLs in the package are **relative**, so it does not depend on the directory name
it is installed under. The built-in UI (`npm run build`) is unchanged.

## Install / update (operator)
1. Unpack the zip into `<config root>/ui/<name>/` (the directory that holds `config_custom.yaml`; for `mosdns start -d /cus/mosdns` that is `/cus/mosdns/ui/<name>/`):
   ```sh
   mkdir -p /cus/mosdns/ui/pg && unzip -o mosdns-ui-*.zip -d /cus/mosdns/ui/pg
   ```
2. **Restart once** so the program checks the package (`POST /api/v1/system/restart`, or the "重启 MosDNS" button; it keeps the process id and takes about 0.4 s). Mounts are registered at start-up.
3. Open `http://<host>:<api port>/pg/`. The built-in page (`/`) shows "检测到外置 UI「pg」版本 X，前往 /pg/" on the system page for every mounted package.

Replacing files of an already mounted package takes effect on the next request (files are re-read live). A *new* package version's manifest is judged — and its version shown in
`GET /api/v1/system/health` → `external_ui` — only at the next start, so restart once after every update.

## Which directories are "mosdns UI packages"
Only a directory that contains `ui-manifest.json` is treated as a mosdns UI package and checked for compatibility. A directory **without** a manifest is served as plain static
pages exactly as before (so other static pages you keep under `ui/` are unaffected); health lists it as `kind: "static"` with the note "非 mosdns UI 包（无 manifest）".

## When a package does not fit
`ui-manifest.json` says which interface level the UI was built for. If it is broken, or the UI needs a newer interface than this program provides (or is older than the program still
supports), or `index.html` is missing, the package is **not mounted**: `/<name>/` is a 404, the built-in `/` keeps working, and `GET /api/v1/system/health` shows
`{"name": "pg", "mounted": false, "kind": "mosdns_ui", "reason": "…"}` (the built-in system page shows the reason too). Fix by installing a matching package or updating the program.

## Roll back
Delete or rename `<config root>/ui/<name>` (and restart once), or simply keep using `/`. Nothing else changes.
