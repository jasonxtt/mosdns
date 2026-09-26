#!/usr/bin/env python3
"""Prospective M4 controls, optionally its fixed first W1 gate."""
import importlib.util
import json
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location('control_gate', Path(__file__).with_name('qualify-m2-controls.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

if __name__ == '__main__':
    try:
        groups = gate.GROUPS
        if len(sys.argv) == 3 and sys.argv[2] == '--w1':
            groups = groups[:2]
        elif len(sys.argv) != 2:
            raise ValueError('usage: qualify-m4-controls.py ROOT [--w1]')
        result = gate.qualify(Path(sys.argv[1]), groups, 'm4')
    except (OSError, ValueError, KeyError, IndexError) as error:
        result = {'qualified': False, 'failures': [str(error)], 'intervals': []}
    print(json.dumps(result, indent=2, allow_nan=False))
    sys.exit(0 if result['qualified'] else 2)
