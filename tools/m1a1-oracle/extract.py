#!/usr/bin/env python3
"""Prepare the M1-a1 differential run from a configuration tree.

Writes, next to `--out`:
  native_extract.yaml   the provider/mapper definitions of the tree plus the
                        minimum (forward, entry sequence, listener) the native
                        compiler needs; relative paths resolve against the
                        configuration root passed to the harness.
  rules_request.jsonl   one `rules` request per provider, for the Go oracle.
  tags.json             provider, mapper and IP-set tags.
"""
import argparse
import json
import os

import yaml

PROVIDERS = {"domain_set_light", "sd_set_light", "adguard_rule"}
IP_SETS = {"si_set"}
MAPPERS = {"domain_mapper"}


def load(root, config):
    main_path = os.path.join(root, config)
    with open(main_path, encoding="utf-8") as handle:
        main = yaml.safe_load(handle) or {}
    defs = []
    for include in main.get("include") or []:
        with open(os.path.join(os.path.dirname(main_path), include), encoding="utf-8") as handle:
            defs.extend((yaml.safe_load(handle) or {}).get("plugins") or [])
    defs.extend(main.get("plugins") or [])
    return defs


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--config", default="config_custom.yaml")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)

    defs = load(args.root, args.config)
    wanted = [d for d in defs if d.get("type") in PROVIDERS | IP_SETS | MAPPERS]
    document = {
        "log": {"level": "error"},
        "plugins": wanted
        + [
            {"tag": "m1a1_fwd", "type": "forward", "args": {"upstreams": [{"addr": "udp://127.0.0.1:15453"}]}},
            {"tag": "m1a1_entry", "type": "sequence", "args": [{"exec": "$m1a1_fwd"}]},
            {"tag": "m1a1_udp", "type": "udp_server", "args": {"entry": "m1a1_entry", "listen": "127.0.0.1:15353"}},
        ],
    }
    with open(os.path.join(args.out, "native_extract.yaml"), "w", encoding="utf-8") as handle:
        yaml.safe_dump(document, handle, allow_unicode=True, sort_keys=False)
    tags = {
        "providers": [d["tag"] for d in wanted if d["type"] in PROVIDERS],
        "ipsets": [d["tag"] for d in wanted if d["type"] in IP_SETS],
        "mappers": [d["tag"] for d in wanted if d["type"] in MAPPERS],
    }
    with open(os.path.join(args.out, "tags.json"), "w", encoding="utf-8") as handle:
        json.dump(tags, handle, indent=1)
    with open(os.path.join(args.out, "rules_request.jsonl"), "w", encoding="utf-8") as handle:
        for tag in tags["providers"]:
            handle.write(json.dumps({"op": "rules", "tag": tag}) + "\n")
    print(json.dumps({k: len(v) for k, v in tags.items()}))


if __name__ == "__main__":
    main()
