# lite-v0.1.6

## Summary

`lite-v0.1.6` syncs the latest upstream socks override behavior into the lite branch.

- `foreign` and `foreignecs` upstream rows now expose an explicit `use_socks_proxy` switch
- historical override data stays compatible and is inferred automatically on load
- `foreignecs` now inherits the global SOCKS5 fallback when proxying remains enabled

## Fixed

- added `use_socks_proxy` to upstream override API payloads and compatibility inference
- allowed `foreign` and `foreignecs` rows to disable SOCKS explicitly without losing backward compatibility for older saved JSON
- extended runtime SOCKS5 fallback from `foreign` to `foreignecs`
- updated the lite upstream editor so the socks toggle appears only for the relevant groups and hides the per-row socks field when proxying is disabled

## Tests

- validated with `go test ./coremain`
- rebuilt the maintained Vue UI bundle with `npm run build`
- deployed the linux/amd64 test binary to `192.168.2.2`
- verified on `192.168.2.2` that:
  - historical `foreign` / `foreignecs` rows load back with `use_socks_proxy=true`
  - both groups inherit the global SOCKS5 proxy at runtime by default
  - setting `use_socks_proxy=false` removes runtime SOCKS5 for that row immediately after save
  - restoring the original override JSON re-enables the expected fallback behavior

## Upgrade Notes

- this release does **not** require a YAML config change
- existing deployments can update only the binary
