# lite-v0.1.9

## Summary

This release fixes dedicated upstream priority for lite deployments when a
domain also matches the DDNS list.

## Fixed

- `sequence_6666` now executes `$sequence_special_all` immediately after the
  special-group matcher and before DDNS and ordinary routing
- removed the duplicate special-group dispatcher calls from the lite A/AAAA
  and other-record processing sequences
- retained lite's `sequence_special_all` configuration contract instead of
  adopting main's `sequence_special` or config schema 4

## Config Package

Use the matching lite full-config package:

<https://raw.githubusercontent.com/jasonxtt/file/28c6493/mosdns/config/config_lite_all.zip>

## Tests

- `go test ./...`
- `cd webui-log && npm run build`
- `git diff --check`

## Upgrade Notes

The priority fix requires both the `lite-v0.1.9` binary and the matching
`config_lite_all.zip` package. Existing lite deployments should replace the
lite full config package before testing a domain that matches both DDNS and a
dedicated upstream group.
