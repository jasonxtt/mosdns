"""Task-owned Linux network isolation. Never attaches a physical NIC to a netns."""
import argparse
import ipaddress
import json
import pathlib
import subprocess

NS = "shadow1-replay"
HOST = "shadow1-h"
PEER = "shadow1-p"
TABLE = "1053"
PRIORITY = "10053"
MARK = "0x1053"


def run(*args):
    return subprocess.check_output(list(args), text=True)


def setup(root, target):
    target = str(ipaddress.IPv4Address(target))
    if (root / "network.json").exists():
        raise RuntimeError("network already initialized; inspect existing state")
    # A physical egress drop guard must already exist before this setup.
    run("nft", "list", "table", "netdev", "shadow1_guard")
    routes = json.loads(run("ip", "-j", "route", "show", "table", "all"))
    if run("ip", "rule", "show").find(PRIORITY + ":") >= 0 or any(str(r.get("table")) == TABLE for r in routes):
        raise RuntimeError("policy table/priority already used")
    local = json.loads(run("ip", "-j", "address", "show"))
    addresses = {x["local"] for dev in local for x in dev.get("addr_info", [])}
    if target not in addresses:
        raise RuntimeError("target must already belong to test host")
    rp = pathlib.Path("/proc/sys/net/ipv4/conf/all/rp_filter")
    state = {"target": target, "rp_filter_all_before": rp.read_text().strip(), "phase": "preparing"}
    (root / "network.json").write_text(json.dumps(state, indent=2) + "\n")
    run("ip", "netns", "add", NS)
    run("ip", "link", "add", HOST, "type", "veth", "peer", "name", PEER)
    run("ip", "link", "set", PEER, "netns", NS)
    run("ip", "link", "set", HOST, "mtu", "65535")
    run("ip", "-n", NS, "link", "set", PEER, "mtu", "65535")
    run("ip", "addr", "add", "192.0.2.1/30", "dev", HOST)
    run("ip", "link", "set", HOST, "up")
    run("ip", "-n", NS, "link", "set", "lo", "up")
    run("ip", "-n", NS, "addr", "add", "192.0.2.2/30", "dev", PEER)
    run("ip", "-n", NS, "link", "set", PEER, "up")
    hostmac = json.loads(run("ip", "-j", "link", "show", HOST))[0]["address"]
    peermac = json.loads(run("ip", "-n", NS, "-j", "link", "show", PEER))[0]["address"]
    run("ip", "neigh", "replace", "192.0.2.2", "lladdr", peermac, "nud", "permanent", "dev", HOST)
    rp.write_text("0\n")
    pathlib.Path("/proc/sys/net/ipv4/conf/" + HOST + "/rp_filter").write_text("0\n")
    run("ip", "route", "add", "table", TABLE, "default", "via", "192.0.2.2", "dev", HOST, "onlink")
    run("ip", "rule", "add", "priority", PRIORITY, "fwmark", MARK, "lookup", TABLE)
    rules = f'''table inet shadow1 {{
 chain replies {{
  type route hook output priority mangle; policy accept;
  meta nfproto ipv4 udp sport 53 fib daddr type != local meta mark set {MARK}
  meta nfproto ipv4 tcp sport 53 fib daddr type != local meta mark set {MARK}
 }}
 chain inject {{
  type filter hook input priority -200; policy accept;
  iifname "{HOST}" ip daddr {target} udp dport 53 accept
  iifname "{HOST}" counter drop
 }}
 chain no_forward {{
  type filter hook forward priority -200; policy accept;
  iifname "{HOST}" counter drop
 }}
}}
'''
    (root / "isolation.nft").write_text(rules)
    run("nft", "-f", str(root / "isolation.nft"))
    # Catch forged-source packets at the physical NIC as an independent backstop.
    run("nft", "add", "rule", "netdev", "shadow1_guard", "physical_out", "ip", "saddr", "!=", target, "counter", "drop")
    state.update(phase="ready", host_mac=hostmac, peer_mac=peermac)
    (root / "network.json").write_text(json.dumps(state, indent=2) + "\n")
    print("network ready; synthetic probe required before real replay")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("--target", required=True)
    args = parser.parse_args()
    setup(args.root, args.target)
