# Cumulative review round 1

Base ac629018d2aa8fd2a34a26f1f7ff0b2ecab4c0fc → ca35be6f7bf1c49b41de0b6640c7366f4382b864. Dedicated C2C completed after14m45s.

P2-1 open: RulesManager loadDiversionRules clears retained catalog on capability refresh failure after accepted mutation. P3-1 open: external_ui slash redirects use decoded filesystem names rather than percent-encoded URL path. Prior slice findings remain closed. FINAL: FAIL

Required regressions: accepted native diversion mutation then capability500/network/invalid retains catalog/disabled controls/retry; external root and nested UTF8/reserved-character directory redirect with safe query follows to200.
