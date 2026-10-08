#!/usr/bin/env python3
"""Measure what the boundary output filter reclaims across local telemetry.

Reads the phase-run store (default ~/.rexymcp/telemetry/phase_runs.jsonl),
skips architect-ledger rows, and sums `output_filtered_tokens` against
`input_tokens` for every run that recorded both.

    python3 docs/dev/issues/scripts/output-filter-savings.py [path]
"""
import json
import os
import statistics
import sys

path = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser(
    "~/.rexymcp/telemetry/phase_runs.jsonl"
)


def find(obj, name):
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k == name:
                return v
            hit = find(v, name)
            if hit is not None:
                return hit
    return None


runs = [
    r
    for r in (json.loads(l) for l in open(path) if l.strip())
    if r.get("record") != "architect_ledger"
]
filtered, inputs = [], []
for r in runs:
    f = find(r, "output_filtered_tokens")
    i = find(r, "input_tokens")
    if f is None or i is None:
        continue
    filtered.append(f)
    inputs.append(i)

ratios = [f / i for f, i in zip(filtered, inputs) if i]
print(f"phase runs: {len(runs)}; with both fields: {len(filtered)}")
print(f"filtered tokens: {sum(filtered)}; input tokens: {sum(inputs)}")
print(f"share of input reclaimed: {100 * sum(filtered) / sum(inputs):.3f}%")
if ratios:
    ratios.sort()
    print(
        f"per-run share: median {100 * statistics.median(ratios):.3f}%  "
        f"p90 {100 * ratios[int(len(ratios) * 0.9)]:.3f}%  "
        f"max {100 * ratios[-1]:.2f}%"
    )
print(f"runs where the filter reclaimed nothing: {sum(1 for f in filtered if not f)}")
