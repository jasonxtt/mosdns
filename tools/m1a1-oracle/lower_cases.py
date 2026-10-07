#!/usr/bin/env python3
import json
import sys

with open(sys.argv[1]) as src, open(sys.argv[2], "w") as dst:
    for line in src:
        case = json.loads(line)
        if "name" in case:
            case["name"] = case["name"].lower()
        dst.write(json.dumps(case) + "\n")
