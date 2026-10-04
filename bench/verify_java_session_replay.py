#!/usr/bin/env python3
"""Verify exact cold/warm source and report replay from java_session_bench."""
import argparse
import hashlib
import json
import pathlib
import re


def inventory(root):
    return {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.rglob("*.java"))
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()
    rows = {}
    for line in (args.root / "summary.txt").read_text().splitlines():
        if line.startswith("phase="):
            row = dict(re.findall(r"(\w+)=([^\s]+)", line))
            row["stats"] = dict(
                re.findall(r"(\w+): (\d+|true|false)", line.split("stats=", 1)[1])
            )
            rows[row["phase"]] = row
    if set(rows) != {"cold", "warm"}:
        raise SystemExit("benchmark did not complete both phases")
    cold = inventory(args.root / "cold" / "sources")
    warm = inventory(args.root / "warm" / "sources")
    missing = sorted(cold.keys() - warm.keys())
    added = sorted(warm.keys() - cold.keys())
    changed = sorted(path for path in cold.keys() & warm.keys() if cold[path] != warm[path])
    reports_equal = (args.root / "cold" / "report-debug.txt").read_bytes() == (
        args.root / "warm" / "report-debug.txt"
    ).read_bytes()
    stats = rows["warm"]["stats"]
    cold_s = float(rows["cold"]["export_wall_s"])
    warm_s = float(rows["warm"]["export_wall_s"])
    valid = (
        not missing and not added and not changed and reports_equal
        and rows["cold"]["checksum"] == rows["warm"]["checksum"]
        and len(cold) == int(rows["cold"]["sources"])
        and len(warm) == int(rows["warm"]["sources"])
        and stats.get("renders") == "1" and stats.get("replays") == "1"
        and stats.get("ready") == "true"
        and rows["cold"]["bytes"] == rows["warm"]["bytes"]
        and stats.get("source_text_bytes") == rows["warm"]["bytes"]
    )
    result = {
        "root": str(args.root), "verified": valid,
        "cold_export_s": cold_s, "warm_export_s": warm_s,
        "replay_speedup": cold_s / warm_s,
        "cold_files": len(cold), "warm_files": len(warm),
        "source_text_bytes": int(rows["warm"]["bytes"]),
        "missing_paths": missing, "added_paths": added, "changed_files": changed,
        "reports_equal": reports_equal, "phase_metrics": rows,
        "cold_speedup_claimed": False, "whole_app_compilation_claimed": False,
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key != "phase_metrics"}))
    if not valid:
        raise SystemExit("source-session replay differs from the original render")


if __name__ == "__main__":
    main()
