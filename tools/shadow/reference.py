"""Low-frequency read-only reference process sampling from the controller.

Run on the controller, with output outside the public repository. Does not
install a sampler or write anything on the reference host.
"""
import argparse
import json
import os
import pathlib
import shlex
import signal
import subprocess
import time

REMOTE = '''import json,os,pathlib,subprocess,time
props=dict(line.split("=",1) for line in subprocess.check_output(["systemctl","show","mosdns.service","-p","MainPID","-p","ActiveState","-p","NRestarts"],text=True).splitlines())
pid=int(props["MainPID"])
p=pathlib.Path("/proc")/str(pid)
stat=(p/"stat").read_text().rsplit(")",1)[1].split()
status={}
for line in (p/"status").read_text().splitlines():
 key,_,value=line.partition(":")
 if key in ("VmRSS","VmHWM","Threads"): status[key]=int(value.split()[0])
print(json.dumps({"wall":time.time(),"monotonic":time.monotonic(),"service":props,"pid":pid,"start_ticks":int(stat[19]),"cpu_seconds":(int(stat[11])+int(stat[12]))/os.sysconf("SC_CLK_TCK"),"rss_kib":status.get("VmRSS"),"hwm_kib":status.get("VmHWM"),"threads":status.get("Threads"),"fd_count":len(list((p/"fd").iterdir()))}))
'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ssh-host", required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--seconds", type=int, default=3800)
    parser.add_argument("--once", action="store_true", help="one best-effort launchd sample; append numerical output")
    parser.add_argument("--end-wall", type=float, default=0)
    args = parser.parse_args()
    if args.once:
        if args.end_wall and time.time() > args.end_wall:
            return
        start = time.time()
        try:
            result = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", args.ssh_host,
                "python3 -c " + shlex.quote(REMOTE)], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
            row = json.loads(result.stdout) if result.returncode == 0 else {"sample_error": "ssh exit", "exit_code": result.returncode}
        except (OSError, ValueError, subprocess.TimeoutExpired) as error:
            row = {"sample_error": type(error).__name__}
        row.update(controller_start=start, controller_end=time.time())
        with args.output.open("a") as stream:
            stream.write(json.dumps(row) + "\n")
        return
    if args.output.exists():
        raise RuntimeError("refuse to overwrite reference evidence")
    args.output.with_suffix(".pid").write_text(str(os.getpid()) + "\n")
    stopped = False

    def stop(*_):
        nonlocal stopped
        stopped = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    deadline, next_sample = time.monotonic() + args.seconds, time.monotonic()
    with args.output.open("x") as stream:
        while not stopped and time.monotonic() <= deadline:
            start = time.time()
            try:
                result = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", args.ssh_host,
                    "python3 -c " + shlex.quote(REMOTE)], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
                if result.returncode:
                    row = {"sample_error": "ssh exit", "exit_code": result.returncode}
                else:
                    row = json.loads(result.stdout)
            except (OSError, ValueError, subprocess.TimeoutExpired) as error:
                row = {"sample_error": type(error).__name__}
            row.update(controller_start=start, controller_end=time.time())
            stream.write(json.dumps(row) + "\n")
            stream.flush()
            next_sample += 60
            while not stopped and time.monotonic() < min(next_sample, deadline):
                time.sleep(min(1, max(0, min(next_sample, deadline) - time.monotonic())))


if __name__ == "__main__":
    main()
