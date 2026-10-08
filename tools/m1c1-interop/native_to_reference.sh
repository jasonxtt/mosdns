#!/bin/sh
# Native host writes domain_output files; the reference plugin reads them back
# (M1-c1 interop evidence). Usage: native_to_reference.sh <mosdns-binary> <m1c1-interop-binary> <workdir>
set -eu
MOSDNS=$1; INTEROP=$2; WORK=$3
rm -rf "$WORK"; mkdir -p "$WORK/gen"
cat > "$WORK/config.yaml" <<YAML
log: {level: error}
api: {http: "127.0.0.1:19099"}
plugins:
  - tag: fwd
    type: forward
    args: {upstreams: [{addr: "udp://127.0.0.1:9"}]}
  - tag: top_domains
    type: domain_output
    args: {enable_flags: true, file_stat: gen/top.txt, file_rule: gen/toprule.txt, dump_interval: 3600}
  - tag: entry
    type: sequence
    args:
      - exec: \$top_domains
      - exec: reject 3
  - tag: listener
    type: udp_server
    args: {entry: entry, listen: "127.0.0.1:19053"}
YAML
cd "$WORK"
"$MOSDNS" start -d "$WORK" -c "$WORK/config.yaml" > mosdns.log 2>&1 &
PID=$!
sleep 1
python3 - <<'PY'
import socket, struct
def q(name, flags):
    out = struct.pack(">HHHHHH", 7, 0x0100 | flags, 1, 0, 0, 0)
    for label in name.split("."):
        out += bytes([len(label)]) + label.encode()
    return out + b"\x00" + struct.pack(">HH", 1, 1)
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.settimeout(3)
for name, flags, times in [("alpha.example", 0, 3), ("beta.example", 0, 2), ("beta.example", 0x20, 1), ("gamma.example", 0x30, 2), ("Mixed.Example", 0, 1)]:
    for _ in range(times):
        s.sendto(q(name, flags), ("127.0.0.1", 19053)); s.recvfrom(1500)
PY
python3 - <<'PY'
import urllib.request
print(urllib.request.urlopen("http://127.0.0.1:19099/plugins/top_domains/save").read())
body = urllib.request.urlopen("http://127.0.0.1:19099/plugins/top_domains/show").read().decode()
open("native_show.txt", "w").write(body)
PY
kill -TERM $PID; wait $PID || true
echo "--- native-written stat file"; cat gen/top.txt
echo "--- native-written rule file"; cat gen/toprule.txt
"$INTEROP" -root "$WORK" -stat gen/top.txt -flags -path "/show?limit=100" > reference_show.txt
echo "--- reference /show of the native file"; cat reference_show.txt
python3 - <<'PY'
def rows(text):
    out = []
    for line in text.strip().splitlines():
        c, d, k = line.split(" ")
        out.append((-int(c), k, d))
    return sorted(out)
native = rows(open("native_show.txt").read())
ref = rows(open("reference_show.txt").read().split("\n", 2)[2])
print("IDENTICAL" if native == ref else "DIFFERENT", len(native), len(ref))
raise SystemExit(0 if native == ref else 1)
PY
