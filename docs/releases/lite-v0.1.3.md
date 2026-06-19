# lite-v0.1.3

## Summary

This release adds dedicated listen ports for lite special routing groups and refines the maintained Vue UI management flow. It also includes query-log audit memory optimizations for long-running instances.

## Added

- added optional `listen_port` support for each special routing group
- added optional `custom_port_only` mode so a special group can be excluded from the default port 53 flow and only serve its custom listen port
- generated dedicated UDP and TCP listeners for special groups that configure a custom port
- added frontend validation for custom special-group ports, including reserved port 53 protection

## Changed

- updated the upstream settings page special-group module to match the main branch card-style manager
- special groups now show custom port state, port 53 participation, and bound upstream count in the manager modal
- the rules manager special-group table now exposes listen-port and port-53 state directly
- retained the lite runtime flow around `sequence_special_all` instead of switching to the main branch split sequence layout

## Improved

- reduced audit-log memory retention by clearing pooled query-context references before returning audit contexts to the pool
- improved log clearing so retained backing arrays are released
- reduced allocation pressure in V2 window statistics by avoiding full log snapshot copies

## Tests

- added special-group config rendering and normalization tests for custom listen ports and custom-port-only behavior
- added audit-log regression tests for memory-release and low-allocation window statistics behavior
- validated locally with `go test ./coremain` and a release binary smoke test on the lite test host

## Config impact

Existing deployments continue to work without YAML changes.

Operators who want per-group dedicated DNS entry points can set a custom listen port in the WebUI. Leaving the port empty preserves the existing port 53 behavior.
