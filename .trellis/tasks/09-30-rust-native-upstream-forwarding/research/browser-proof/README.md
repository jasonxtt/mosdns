# Native Vue browser proof

This proof uses only the isolated `mosdns-rust` checkout and loopback peers.
It exercises the built native host's real `/api/v2/audit/logs` response through
the Vite proxy and opens the maintained Vue page in a browser.

From the remote checkout, after the native build and disposable `npm ci`:

```sh
cd /root/mosdns-rust-forwarding.8UEU53
python3 .trellis/tasks/09-30-rust-native-upstream-forwarding/research/browser-proof/upstream.py > browser-proof-upstream.log 2>&1 &
UPSTREAM_PID=$!
./rust/target/debug/mosdns start -c .trellis/tasks/09-30-rust-native-upstream-forwarding/research/browser-proof/native-browser.yaml > browser-proof-native.log 2>&1 &
NATIVE_PID=$!
cd webui-log
MOSDNS_DEV_TARGET=http://127.0.0.1:18080 npm run dev -- --host 127.0.0.1 --port 14173 > ../browser-proof-vite.log 2>&1 &
VITE_PID=$!
```

In a local terminal, forward only those loopback ports:

```sh
ssh -o ExitOnForwardFailure=yes -N \
  -L 14173:127.0.0.1:14173 \
  -L 18080:127.0.0.1:18080 mosdns-rust
```

Open `http://127.0.0.1:14173/`, send a query through the native UDP listener
(`127.0.0.1:15400`) from the remote checkout, and open the query detail. The
browser assertion is that the returned real log shows schema version 1 with
the `primary` entry, peer `127.0.0.1:15453`, UDP transport, and an ordered
response attempt. The same log remains available at the proxied
`/api/v2/audit/logs` endpoint.

Cleanup is restricted to the owned processes and forwarded session:

```sh
kill "$VITE_PID" "$NATIVE_PID" "$UPSTREAM_PID"
```

No production service, public DNS, or shared listener is used.
