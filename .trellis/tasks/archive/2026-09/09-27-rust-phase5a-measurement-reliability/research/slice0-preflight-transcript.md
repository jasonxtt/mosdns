# Slice 0 preflight transcript

This artifact closes C2C finding P2-1 from the first Slice 0 review. It
preserves the exact read-only remote command, raw stdout, and wrapper exit
status. The command was rerun on 2026-09-27 local time at
`2026-09-26T17:06:41Z`; it did not write remote files, change settings, start
processes, or access `ssh mos`.

## Exact invocation

The outer shell was run from the repository with `set +e`; its only purpose was
to retain the SSH exit code without hiding a failed read-only probe:

```sh
ssh mosdns-rust 'set -eu
printf "captured_utc="; date -u +%Y-%m-%dT%H:%M:%SZ
printf "hostname="; hostname
printf "kernel="; uname -srvm
printf "arch="; uname -m
printf "cpus="; getconf _NPROCESSORS_ONLN
printf "cpu_online="; cat /sys/devices/system/cpu/online
printf "affinity="; taskset -pc $$ 2>&1
printf "loadavg="; cat /proc/loadavg
printf "mem_total_kb="; awk "/^MemTotal:/{print \$2}" /proc/meminfo
printf "mem_available_kb="; awk "/^MemAvailable:/{print \$2}" /proc/meminfo
printf "root_fs="; df -P -T / | tail -n 1
printf "tmp_fs="; df -P -T /tmp | tail -n 1
printf "fd_limit="; ulimit -n
printf "cgroup_mount="; findmnt -n -o FSTYPE,TARGET /sys/fs/cgroup 2>/dev/null || true
printf "cpu_max="; if test -r /sys/fs/cgroup/cpu.max; then cat /sys/fs/cgroup/cpu.max; else echo unavailable; fi
printf "cpuset_effective="; if test -r /sys/fs/cgroup/cpuset.cpus.effective; then cat /sys/fs/cgroup/cpuset.cpus.effective; else echo unavailable; fi
printf "port_range="; cat /proc/sys/net/ipv4/ip_local_port_range
printf "tcp_tw_reuse="; cat /proc/sys/net/ipv4/tcp_tw_reuse
printf "tcp_fin_timeout="; cat /proc/sys/net/ipv4/tcp_fin_timeout
printf "time_wait="; ss -tan state time-wait | tail -n +2 | wc -l
printf "established="; ss -tan state established | tail -n +2 | wc -l
printf "listeners="; ss -ltnup | awk "NR==1 || /:53 |:7777 |:2222 |:4444 |:8888 |:3077 |:3099 |:3111/"
printf "clktck="; getconf CLK_TCK
printf "perf_event_paranoid="; cat /proc/sys/kernel/perf_event_paranoid
printf "cap_eff="; awk "/^CapEff:/{print \$2}" /proc/self/status
printf "tools="; for x in perf strace bpftrace gdb valgrind eu-stack addr2line objdump nm readelf go rustc cargo taskset ss; do if command -v "$x" >/dev/null 2>&1; then printf "%s=%s;" "$x" "$(command -v "$x")"; else printf "%s=missing;" "$x"; fi; done; printf "\n"
printf "go_version="; go version
printf "rust_version="; rustc --version
printf "cargo_version="; cargo --version
printf "proc_self_status="; test -r /proc/self/status && echo readable
'
rc=$?
printf 'ssh_exit_status=%s\n' "$rc"
exit "$rc"
```

## Raw stdout

```text
captured_utc=2026-09-26T17:06:41Z
hostname=mosdns-rust
kernel=Linux 7.0.9-x64v3-xanmod1 #0~20260517.ga456799 SMP PREEMPT_DYNAMIC Sun May 17 20:10:49 UTC x86_64
arch=x86_64
cpus=2
cpu_online=0-1
affinity=pid 354109's current affinity list: 0,1
loadavg=0.12 0.09 0.09 1/115 354117
mem_total_kb=4006424
mem_available_kb=3420992
root_fs=/dev/sda1      ext4    24560264 21210316   2302112      91% /
tmp_fs=tmpfs          tmpfs     2003212   208   2003004       1% /tmp
fd_limit=1024
cgroup_mount=cgroup2 /sys/fs/cgroup
cpu_max=unavailable
cpuset_effective=0-1
port_range=32768\t60999
tcp_tw_reuse=2
tcp_fin_timeout=60
time_wait=0
established=1
listeners=Netid State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess
udp UNCONN 0 0 *:7777 *:* users:("mosdns",pid=425,fd=13)
udp UNCONN 0 0 *:53 *:* users:("mosdns",pid=425,fd=17)
udp UNCONN 0 0 *:2222 *:* users:("mosdns",pid=425,fd=11)
udp UNCONN 0 0 *:4444 *:* users:("mosdns",pid=425,fd=9)
udp UNCONN 0 0 *:8888 *:* users:("mosdns",pid=425,fd=15)
udp UNCONN 0 0 *:3077 *:* users:("mosdns",pid=425,fd=19)
udp UNCONN 0 0 *:3099 *:* users:("mosdns",pid=425,fd=21)
udp UNCONN 0 0 *:3111 *:* users:("mosdns",pid=425,fd=23)
tcp LISTEN 0 4096 *:8888 *:* users:("mosdns",pid=425,fd=16)
tcp LISTEN 0 4096 *:2222 *:* users:("mosdns",pid=425,fd=12)
tcp LISTEN 0 4096 *:53 *:* users:("mosdns",pid=425,fd=18)
tcp LISTEN 0 4096 *:4444 *:* users:("mosdns",pid=425,fd=10)
tcp LISTEN 0 4096 *:7777 *:* users:("mosdns",pid=425,fd=14)
tcp LISTEN 0 4096 *:3111 *:* users:("mosdns",pid=425,fd=24)
tcp LISTEN 0 4096 *:3099 *:* users:("mosdns",pid=425,fd=22)
tcp LISTEN 0 4096 *:3077 *:* users:("mosdns",pid=425,fd=20)
clktck=100
perf_event_paranoid=2
cap_eff=000001ffffffffff
tools=perf=missing;strace=missing;bpftrace=missing;gdb=missing;valgrind=missing;eu-stack=missing;addr2line=/usr/bin/addr2line;objdump=/usr/bin/objdump;nm=/usr/bin/nm;readelf=/usr/bin/readelf;go=/usr/bin/go;rustc=/root/.cargo/bin/rustc;cargo=/root/.cargo/bin/cargo;taskset=/usr/bin/taskset;ss=/usr/bin/ss;
go_version=go version go1.24.4 linux/amd64
rust_version=rustc 1.95.0 (59807616e 2026-04-14)
cargo_version=cargo 1.95.0 (f2d3ce0bd 2026-03-21)
proc_self_status=readable
ssh_exit_status=0
```

The transcript is raw command output; the normalized interpretation and the
profiler blocker remain in `slice0-environment-preflight.md`.
