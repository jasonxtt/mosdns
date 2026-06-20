# lite-v0.1.4

## Summary

This release brings the main-branch overview ranking detail experience to the lite WebUI while keeping lite-specific audit and routing behavior intact.

## Added

- added clickable detail views for all overview ranking modules:
  - Top domains
  - Top clients
  - Slowest queries
  - Routing statistics
- added a shared ranking-detail panel for domain, client, and routing-rule rows
- added precise audit-log filters for `domain_set` and `effective_tag`
- added overview-to-query-log filtering so detail panels can jump directly to the live query log with a matching filter

## Changed

- replaced the noisy inline “详情” text in overview rows with a compact arrow hint
- refined the Top clients layout so the client value, arrow hint, and count stay aligned on one row
- expanded ranking-detail tables to include domain, client, query type, routing rule, upstream, latency, response, and per-record detail actions
- improved mobile rendering for ranking-detail tables with card-style `data-label` rows

## Fixed

- fixed routing-statistic details using broad text search by adding exact `effective_tag` / `domain_set` filtering
- fixed Top client detail hints wrapping onto a second line in narrow overview cards

## Tests

- validated with `npm run build`
- validated with `go test ./coremain`
- validated with `go test ./...`
- compiled a linux/amd64 binary and deployed it to the `ssjy-2` test host
- verified `mosdns.service` starts successfully on `ssjy-2`
- verified audit stats, effective ranking, and exact `effective_tag` audit-log filtering on `ssjy-2`

## Config impact

No configuration changes are required. Existing lite deployments continue to use their current runtime configuration.
