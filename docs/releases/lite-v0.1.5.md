# lite-v0.1.5

## Summary

`lite-v0.1.5` fixes audit visibility for `special_groups` custom listen ports in the lite branch.

- custom UDP/TCP listeners generated for dedicated groups now enable audit logging
- queries entering through a dedicated group's custom port can appear in the WebUI query log
- generated-config regression coverage now checks that the custom-port listeners keep `enable_audit: true`

## Fixed

- added `enable_audit: true` to dynamically generated `special_udp_server_*` entries
- added `enable_audit: true` to dynamically generated `special_tcp_server_*` entries
- restored WebUI audit-log visibility for dedicated-group custom-port queries, including `matched_group`, `final_sequence`, and `final_upstream` metadata

## Upgrade Notes

- this is a binary-only lite release
- no YAML config migration is required
- existing deployments can update only the binary
