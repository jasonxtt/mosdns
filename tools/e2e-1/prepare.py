"""Prepare disposable roots; never modify supplied packages or snapshot."""
import json
import pathlib
import re
import shutil
import subprocess
from urllib.parse import urlsplit

import yaml
from mock import GROUPS, MARKERS, ports


def load(path):
    return json.loads(path.read_text())


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def definitions(root):
    main = yaml.safe_load((root / "config_custom.yaml").read_text())
    result = main.get("plugins", [])[:]
    for include in main.get("include", []):
        result.extend(yaml.safe_load((root / include).read_text()).get("plugins", []))
    return result


def mapped(entry, group):
    result = dict(entry)
    protocol = result.get("protocol") or urlsplit(result["addr"]).scheme or "udp"
    protocol = {"dot": "tls", "doh": "https"}.get(protocol, protocol)
    if protocol not in ports(group):
        raise ValueError("unsupported enabled protocol in " + group)
    host = "localhost" if protocol in ("https", "tls") else "127.0.0.1"
    result["addr"] = f"{protocol}://{host}:{ports(group)[protocol]}" + ("/dns-query" if protocol == "https" else "")
    result["protocol"] = protocol
    if protocol in ("https", "tls"):
        result["dial_addr"] = f"127.0.0.1:{ports(group)[protocol]}"
    else:
        result.pop("dial_addr", None)
    result.pop("bootstrap", None)
    if result.get("socks5"):
        result["socks5"] = "127.0.0.1:17891"
    return result


def prepare(work, inputs, migrate, count):
    snapshot = inputs / "snapshot"
    for role, package in (("go", "config_all"), ("native", "config_rust")):
        root = work / role
        shutil.copytree(inputs / package, root)
        for directory in ("rule", "gen", "srs", "adguard", "webinfo"):
            shutil.rmtree(root / directory, ignore_errors=True)
            shutil.copytree(snapshot / directory, root / directory)
        # Privacy: ignored Finder metadata is not runtime input.
        for path in root.rglob("._*"):
            path.unlink()
        (root / "cache").mkdir(exist_ok=True)
        for path in (root / "cache").glob("*.dump"):
            path.unlink()
    native, go = work / "native", work / "go"
    migrated = work / "migrated"
    # This happens BEFORE any endpoint replacement.
    result = subprocess.run([str(migrate), "--from", str(snapshot), "-d", str(inputs / "config_rust"), "-c", "config_custom.yaml", "--out", str(migrated)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (work / "migration-output.txt").write_bytes(result.stdout)
    if result.returncode:
        raise RuntimeError("migration requires a decision; see private migration-output.txt")
    settings = load(migrated / "settings.json")
    settings.setdefault("proxies", {}).setdefault("main_proxy", {})["addr"] = "127.0.0.1:17891"
    # ECS retains its actual value in VM-private settings; never publish it.
    native_groups = {d["tag"]: d["args"] for d in definitions(native) if d["type"] == "aliapi"}
    if set(native_groups) != set(GROUPS):
        raise ValueError("unexpected upstream group needs a mapping")
    for group, args in native_groups.items():
        entries = settings.get("upstreams", {}).get(group)
        if entries is None:
            entries = [dict(e, enabled=True) for e in args["upstreams"]]
        settings.setdefault("upstreams", {})[group] = [mapped(e, group) if e.get("enabled", True) else e for e in entries]
    save(native / "webinfo/settings.json", settings)
    overrides = load(go / "webinfo/upstream_overrides.json")
    for group, entries in overrides.items():
        if group not in GROUPS:
            if any(e.get("enabled") for e in entries):
                raise ValueError("unexpected Go group")
            continue
        if not any(e.get("enabled") for e in entries):
            raise ValueError("Go YAML fallback needs explicit mapping")
        overrides[group] = [mapped(e, group) if e.get("enabled") else e for e in entries]
    save(go / "webinfo/upstream_overrides.json", overrides)
    global_values = load(go / "webinfo/config_overrides.json")
    global_values["socks5"] = "127.0.0.1:17891"
    save(go / "webinfo/config_overrides.json", global_values)
    aliases = {}
    for role, groups in (("go", overrides), ("native", settings["upstreams"])):
        table = {g: [g] for g in GROUPS}
        for group, entries in groups.items():
            if group not in GROUPS:
                continue
            for entry in entries:
                if entry.get("enabled", True) and entry.get("tag"):
                    table.setdefault(entry["tag"], [])
                    if group not in table[entry["tag"]]:
                        table[entry["tag"]].append(group)
        aliases[role] = table
    save(work / "mapping.json", {"groups": {g: {"ports": ports(g), "answers": MARKERS[g]} for g in GROUPS}, "upstream_aliases": aliases})
    names = []
    for line in (snapshot / "gen/top_domains.txt").read_text().splitlines():
        fields = line.split()
        if len(fields) < 2:
            continue
        candidate = fields[2 if len(fields) >= 3 else 1].split("|")[0].rstrip(".").lower()
        if len(candidate) <= 253 and re.fullmatch(r"[a-z0-9_-]+(?:\.[a-z0-9_-]+)+", candidate) and all(len(p) <= 63 for p in candidate.split(".")) and candidate not in names:
            names.append(candidate)
        if len(names) == count:
            break
    if len(names) < count:
        raise ValueError("insufficient production names")
    corpus = [{"name": n, "type": qt, "source": "127.0.0.1", "port": 53, "class": "top"} for n in names for qt in (1, 28)]
    probes = {
        "rule/blocklist.txt": "block", "rule/whitelist.txt": "white",
        "rule/greylist.txt": "grey", "rule/realiplist.txt": "real",
        "rule/ddnslist.txt": "ddns", "rule/cnfakeipfilter.txt": "cnfilter",
        "gen/realiprule.txt": "memory-real", "gen/fakeiprule.txt": "memory-fake",
        "gen/nov4rule.txt": "nov4", "gen/nov6rule.txt": "nov6", "gen/sv4rule.txt": "sv4",
    }
    for root in (go, native):
        for file, probe in probes.items():
            with (root / file).open("a") as handle:
                handle.write(f"\nfull:e2e1-{probe}.example.com\n")
        with (root / "rule/client_ip.txt").open("a") as handle:
            handle.write("\n127.0.0.2/32\n")
        with (root / "rule/rewrite.txt").open("a") as handle:
            handle.write("\nfull:e2e1-rewrite-ip.example.com 192.0.2.77\nfull:e2e1-rewrite-name.example.com e2e1-target.example.com\n")
        manifests = load(root / "adguard/config.json")
        lists = manifests if isinstance(manifests, list) else manifests.get("rules", [])
        enabled = next((e for e in lists if e.get("enabled")), None)
        if enabled:
            with (root / "adguard" / (enabled["id"] + ".rules")).open("a") as handle:
                handle.write("\n||e2e1-adguard.example.com^\n")
    for probe in list(probes.values()) + ["rewrite-ip", "rewrite-name", "adguard", "unmatched"]:
        for source in ("127.0.0.1", "127.0.0.2"):
            for qt in (1, 28):
                corpus.append({"name": f"e2e1-{probe}.example.com", "type": qt, "source": source, "port": 53, "class": probe})
    listener_ports = sorted({int(d["args"]["listen"].rsplit(":", 1)[1]) for d in definitions(native) if d["type"] == "udp_server"})
    for port in listener_ports:
        if port != 53:
            for qt in (1, 28):
                corpus.append({"name": f"e2e1-listener-{port}.example.com", "type": qt, "source": "127.0.0.1", "port": port, "class": "listener"})
    save(work / "corpus.json", corpus)
    save(work / "audit-ports.json", sorted({int(d["args"]["listen"].rsplit(":", 1)[1]) for d in definitions(native) if d["type"] == "udp_server" and d["args"].get("enable_audit")}))
    save(work / "corpus-summary.json", {"production_names": len(names), "queries_per_pass": len(corpus), "listener_ports": listener_ports, "controlled_rule_additions": len(probes) + 4})
