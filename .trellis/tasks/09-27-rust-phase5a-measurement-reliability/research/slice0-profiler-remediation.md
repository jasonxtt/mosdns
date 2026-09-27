# Slice 0 profiler remediation transcript

This is the follow-up to the initial read-only preflight. The user authorized
installation on `mosdns-rust` on 2026-09-27 Asia/Shanghai time. The operation
was limited to profiler packages and controlled smoke workloads; it did not
change kernel policy, restart MosDNS, send DNS traffic, or run a measurement
stage.

## Initial capability recheck

Exact command:

```sh
ssh mosdns-rust 'set -u; printf "user="; id -un; printf "uid="; id -u; printf "host="; hostname; printf "kernel="; uname -srmo; printf "os="; sed -n "1p" /etc/os-release 2>/dev/null || true; printf "package_managers="; for x in apt-get apt dnf yum apk pacman zypper; do if command -v "$x" >/dev/null 2>&1; then printf "%s=%s;" "$x" "$(command -v "$x")"; fi; done; printf "\nperf_event_paranoid="; cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true; printf "cap_eff="; awk "/^CapEff:/{print \$2}" /proc/self/status 2>/dev/null || true; printf "arch="; dpkg --print-architecture 2>/dev/null || uname -m'
```

Observed output:

```text
user=root
uid=0
host=mosdns-rust
kernel=Linux 7.0.9-x64v3-xanmod1 x86_64 GNU/Linux
os=PRETTY_NAME="Debian GNU/Linux 13 (trixie)"
package_managers=apt-get=/usr/bin/apt-get;apt=/usr/bin/apt;
perf_event_paranoid=2
cap_eff=000001ffffffffff
arch=amd64
```

The installed package candidates were `linux-perf 6.12.107-1` and
`libc6-dbg 2.41-12+deb13u4`.

## Installation and policy check

Exact install command:

```sh
ssh mosdns-rust 'apt-get install -y --no-install-recommends linux-perf libc6-dbg'
```

Verification output:

```text
libc6-dbg install ok installed 2.41-12+deb13u4
linux-perf install ok installed 6.12.107-1
perf version 6.12.107
perf_event_paranoid=2
```

No kernel security setting was lowered.

## Controlled profiler smoke

The first smoke command had a Python quoting error and returned
`record_status=1` with no samples. A second draft used an unbounded allocation
pattern; its exact confirmed PIDs were terminated, and no MosDNS or service PID
was touched. These failed harness attempts are retained here rather than
treated as profiler evidence.

The bounded successful command used the software `cpu-clock` event and a fixed
50-million-iteration Python workload:

```sh
ssh mosdns-rust 'smoke_dir=/root/mosdns-rust-phase5a-profile-smoke; smoke_data=$(mktemp "$smoke_dir/perf-cpu-clock-fixed.XXXXXX.data"); record_log="$smoke_data.record.log"; report_log="$smoke_data.report.txt"; /usr/bin/perf record -e cpu-clock -F 99 --call-graph fp --no-buildid-cache --output "$smoke_data" -- /usr/bin/python3 -c '\''sum(i*i for i in range(50000000))'\'' >/dev/null 2>"$record_log"; record_status=$?; /usr/bin/perf report --stdio --input "$smoke_data" --sort comm,dso,symbol --percent-limit 0.1 >"$report_log" 2>&1; report_status=$?; printf "data=%s\\nrecord_status=%s\\nreport_status=%s\\ndata_bytes=" "$smoke_data" "$record_status" "$report_status"; stat -c %s "$smoke_data"; sed -n "1,80p" "$report_log"'
```

Observed result:

```text
record_status=0
report_status=0
data_bytes=48806
[ perf record: Captured and wrote 0.030 MB ... (232 samples) ]
# Total Lost Samples: 0
# Samples: 232 of event 'cpu-clock'
```

The report emitted call-chain entries including `_PyLong_Multiply`,
`PyNumber_Add`, `_PyEval_EvalFrameDefault`, and `PyIter_Next`. The smoke data
SHA-256 is
`5f22bdb5a19e53e5c419fcecc92502678e31ffbf5684a753f73101f8a3a02418` and the
report SHA-256 is
`839b58d038a7a21265221c3cb20fa2e91ae137d4001249d2d02869feee483aef`.

## PMU versus software event

An explicit controlled `perf stat` check returned status zero, but `cycles` and
`instructions` were both reported as zero while `task-clock` was non-zero:

```text
0 cycles
0 instructions
823.59 msec task-clock   # 0.999 CPUs utilized
0.824581679 seconds time elapsed
```

Therefore the host currently proves usable process-directed software-event
sampling (`cpu-clock`) and call-chain export, but not useful hardware PMU
cycle/instruction counts. Future profile runs must record this limitation and
use the approved software-event path unless a later environment review enables
usable PMU data. G0 is pending reviewer acceptance of this remediation; this
transcript alone does not authorize Slice 2 official measurement.
