# lite-v0.1.1

## Summary

This release fixes query-log search behavior for client IPs and client aliases in the maintained Vue UI.

- exact client-IP search now works when operators type plain IPv4 values such as `10.0.0.10`
- stored IPv4-mapped IPv6 values like `::ffff:10.0.0.10` are normalized before exact comparison and dedicated client-IP filtering
- client alias search now supports both fuzzy and exact matching, including aliases that map to multiple client IPs

## Fixed

- fixed audit log exact search so plain IPv4 input matches logs whose stored `client_ip` uses IPv4-mapped IPv6 form
- fixed dedicated `client_ip` filtering to compare normalized addresses instead of raw stored strings
- fixed client alias search flow in the Vue UI so exact and fuzzy alias input both resolve to matching client IPs before loading logs
- fixed alias-backed searches to include every matching client IP instead of collapsing to a single exact alias hit

## Tests

- added audit log regression tests for normalized client-IP comparison and multi-client alias-backed filtering in `coremain/audit_test.go`

## Config impact

This release does **not** require a YAML config change.

Existing deployments can update only the binary.
