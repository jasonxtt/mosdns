# Browser proof record

This proof used only the `mosdns-rust` SSH VM and disposable paths under
`/root/mosdns-rust-querydiag`. The source snapshot was synchronized with
`rsync -a`, excluding `.git`, `target`, `node_modules`, and archived Trellis
tasks. No production service or `/cus/mosdns` path was used.

The first disposable config intentionally exposed an existing native-host
boundary: one host assembly accepts exactly one listener plugin. The browser
proof was corrected by using two isolated native processes (one UDP and one
TCP), each with its own API port; no product code was changed for this setup
correction.

## Topology and resources

- Native UDP process: `mosdns start -c native-browser.yaml`, API `127.0.0.1:18080`, DNS `127.0.0.1:15353`.
- Native TCP process: `mosdns start -c native-browser-tcp.yaml`, API `127.0.0.1:18081`, DNS `127.0.0.1:15354`.
- Disposable Vite: `MOSDNS_DEV_TARGET=http://127.0.0.1:18080 npm run dev -- --host 127.0.0.1 --port 15173`.
- Local tunnel: `ssh -N -L 15173:127.0.0.1:15173 mosdns-rust`.
- The native processes, Vite child, and tunnel were stopped after verification; the
  remote ports were checked clear with `ss`.

## Final-candidate actions and observations

1. `dig @127.0.0.1 -p 15353 browser-ra-fixed.example A` returned a real UDP
   NXDOMAIN response; `dig @127.0.0.1 -p 15354 browser-ra-final-tcp.example A
   +tcp` returned a real TCP NXDOMAIN response. A second disposable run issued
   `browser-finalpass-0.example` through `browser-finalpass-54.example` over
   UDP plus `browser-finalpass-tcp.example` over TCP, giving 55 retained rows.
2. The native UDP API returned rich records with a 51-character
   `n-<nonce>-<counter>` trace ID, `response_code: NXDOMAIN`, `RA` only (no
   false `TC`), empty answers, `answer_details_status: complete`, and
   `final_sequence: browser_reject`.
3. Chrome opened `http://127.0.0.1:15173/` through the owned SSH tunnel. The
   final-candidate QueryManager showed page 1/2 and 55 rows; Load more reached
   page 2, fuzzy search and quoted exact search each returned one row, opening
   the row showed IN, trace ID, NXDOMAIN, RA, complete answer status, and no
   fabricated route/upstream values, and the domain quick-filter action
   refreshed the list.
4. The final-candidate Overview loaded Top domain, Top client, slowest and
   routing/effective panels. Top client showed the raw `127.0.0.1` address
   with an explicit alias-unavailable message; metrics, switch status and
   upstream config showed explicit HTTP 404 unavailable messages. Clicking Top
   domain opened exact `logs/domain` drill-down for
   `browser-finalpass-0.example` with one matching record, and its nested
   `查看` action opened the rich query detail.

This is a focused browser/API/wire proof for the new diagnostics surfaces. It
does not claim full native parity for the unrelated upstream, alias, metrics,
or system-control sections.
