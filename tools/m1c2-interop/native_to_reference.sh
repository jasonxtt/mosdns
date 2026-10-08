#!/bin/sh
# Native host writes the requery state file; the reference plugin reads it back
# (M1-c2 interop evidence). Usage: native_to_reference.sh <mosdns-binary> <m1c2-interop-binary> <workdir>
set -eu
MOSDNS=$1; INTEROP=$2; WORK=$3
rm -rf "$WORK"; mkdir -p "$WORK/webinfo" "$WORK/gen"
cat > "$WORK/webinfo/requeryconfig.json" <<JSON
{"domain_processing":{"source_files":[{"alias":"x","path":"gen/none.txt"}]},"url_actions":{"save_rules":[],"flush_rules":[]},"scheduler":{"enabled":false,"start_datetime":"","interval_minutes":0},"execution_settings":{"queries_per_second":10,"resolver_address":"127.0.0.1:19053","url_call_delay_ms":5,"date_range_days":30},"status":{"task_state":"running","progress":{"processed":3,"total":9}}}
JSON
cat > "$WORK/config.yaml" <<YAML
log: {level: error}
api: {http: "127.0.0.1:19199"}
plugins:
  - tag: fwd
    type: forward
    args: {upstreams: [{addr: "udp://127.0.0.1:9"}]}
  - tag: requery
    type: requery
    args: {file: webinfo/requeryconfig.json}
  - tag: entry
    type: sequence
    args:
      - exec: \$fwd
  - tag: listener
    type: udp_server
    args: {entry: entry, listen: "127.0.0.1:19053"}
YAML
cd "$WORK"
"$MOSDNS" start -d "$WORK" -c "$WORK/config.yaml" > mosdns.log 2>&1 &
PID=$!
sleep 1
python3 - <<'PY'
import json, urllib.request
base = "http://127.0.0.1:19199/plugins/requery"
req = urllib.request.Request(base + "/scheduler/config", method="POST",
    data=json.dumps({"enabled": True, "start_datetime": "2031-02-03T04:05:06.789Z", "interval_minutes": 90, "date_range_days": 21}).encode())
print(urllib.request.urlopen(req).read())
# a run with no source files finishes at once ("no domains")
print(urllib.request.urlopen(urllib.request.Request(base + "/trigger", method="POST")).read())
import time; time.sleep(1)
open("native_root.txt", "w").write(urllib.request.urlopen(base + "/").read().decode())
open("native_status.txt", "w").write(urllib.request.urlopen(base + "/status").read().decode())
PY
kill -TERM $PID; wait $PID || true
echo "--- native-written state file"; cat webinfo/requeryconfig.json; echo
"$INTEROP" -root "$WORK" -file webinfo/requeryconfig.json -path / 2>/dev/null > reference_root.txt
"$INTEROP" -root "$WORK" -file webinfo/requeryconfig.json -path /status 2>/dev/null > reference_status.txt
python3 - <<'PY'
native = open("native_root.txt").read()
ref = open("reference_root.txt").read().split("\n", 1)[1]
status_native = open("native_status.txt").read()
status_ref = open("reference_status.txt").read().split("\n", 1)[1]
same = native == ref and status_native == status_ref
print("IDENTICAL" if same else "DIFFERENT")
if not same:
    print(native); print(ref)
raise SystemExit(0 if same else 1)
PY
