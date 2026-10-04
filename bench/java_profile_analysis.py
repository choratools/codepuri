#!/usr/bin/env python3
"""Analyze elapsed slot accounting and optionally compare source trees exactly."""
import argparse
import csv
import hashlib
import json
import re
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("profile", type=Path)
    parser.add_argument("--baseline-sources", type=Path)
    parser.add_argument("--compare-runs", type=Path, nargs="*", default=[])
    args = parser.parse_args()
    with (args.profile / "classes.tsv").open() as stream:
        classes = list(csv.DictReader(stream, delimiter="\t"))
    with (args.profile / "batches.tsv").open() as stream:
        batches = list(csv.DictReader(stream, delimiter="\t"))
    cursor = 0
    idle_counts = {"over_1ms": 0, "over_10ms": 0, "over_100ms": 0}
    top = []
    dex_work = {}
    dex_longest = {}
    critical_sum = 0.0
    for batch in batches:
        tasks = classes[cursor:cursor + int(batch["tasks"])]
        cursor += len(tasks)
        wall = float(batch["wall_s"])
        capacity = int(batch["capacity"])
        finishes = [float(t["start_s"]) + float(t["elapsed_s"]) for t in tasks]
        for finish in finishes:
            for label, threshold in [("over_1ms", .001), ("over_10ms", .01), ("over_100ms", .1)]:
                idle_counts[label] += wall - finish > threshold
        last = max(tasks, key=lambda t: float(t["start_s"]) + float(t["elapsed_s"]))
        critical_sum += float(last["elapsed_s"])
        dex = int(batch["dex"])
        dex_work[dex] = dex_work.get(dex, 0.0) + float(batch["task_sum_s"])
        dex_longest[dex] = max(dex_longest.get(dex, 0.0), max(float(t["elapsed_s"]) for t in tasks))
        top.append({"dex": dex, "first_class": int(batch["first_class"]), "wall_s": wall,
                    "last_descriptor": last["descriptor"],
                    "finished_before_last_100ms": sum(f < max(finishes) - .1 for f in finishes),
                    "finished_by_half_wall": sum(f <= wall / 2 for f in finishes),
                    "active_task_average": float(batch["task_sum_s"]) / wall,
                    "finished_idle_slots_average": float(batch["finished_idle_s"]) / wall,
                    "capacity": capacity})
    workers = int(batches[0]["capacity"]) if batches else 1
    result = {"classes": len(classes), "batches": len(batches), "finished_idle_task_counts": idle_counts,
              "sum_last_finishing_task_elapsed_s": critical_sum,
              "optimistic_fixed_duration_compute_lower_bound_s": sum(max(work / workers, dex_longest[dex]) for dex, work in dex_work.items()),
              "lower_bound_caveat": "Elapsed durations frozen from this run; excludes dispatch/ordering costs and changes in contention. Not a measured speedup.",
              "top_batches": sorted(top, key=lambda b: b["wall_s"], reverse=True)[:10]}
    if args.baseline_sources:
        current = args.profile / "sources"
        baseline = args.baseline_sources
        a = {p.relative_to(current) for p in current.rglob("*.java")}
        b = {p.relative_to(baseline) for p in baseline.rglob("*.java")}
        different = []
        non_diagnostic_differences = []
        edge_id_only_differences = 0
        digest = hashlib.sha256()
        for relative in sorted(a & b):
            data = (current / relative).read_bytes()
            digest.update(str(relative).encode() + b"\0" + data + b"\0")
            original = (baseline / relative).read_bytes()
            if data != original:
                different.append(str(relative))
                def without_unavailable_comments(value):
                    return b"\n".join(line for line in value.splitlines()
                                      if not line.lstrip().startswith(b"// unavailable original method index "))
                if without_unavailable_comments(data) != without_unavailable_comments(original):
                    non_diagnostic_differences.append(str(relative))
                if re.sub(rb"EdgeId\(\d+v\d+\)", b"EdgeId(*)", data) == re.sub(rb"EdgeId\(\d+v\d+\)", b"EdgeId(*)", original):
                    edge_id_only_differences += 1
        result["source_parity"] = {"current_files": len(a), "baseline_files": len(b),
                                   "missing": sorted(map(str, b - a)), "extra": sorted(map(str, a - b)),
                                   "different_bytes": different, "ordered_sha256": digest.hexdigest(),
                                   "exact_match": a == b and not different,
                                   "different_file_count": len(different),
                                   "edge_id_only_different_files": edge_id_only_differences,
                                   "non_unavailable_comment_differences": non_diagnostic_differences,
                                   "excluding_unavailable_comments_match": a == b and not non_diagnostic_differences}
    if args.compare_runs:
        runs = []
        reports = []
        for path in args.compare_runs:
            lines = (path / "summary.txt").read_text().splitlines()
            values = dict(re.findall(r"(\w+)=([^ ]+)", lines[0]))
            values["artifact"] = str(path)
            runs.append(values)
            reports.append(lines[1])
        result["compared_runs"] = runs
        result["compared_exact_reports_equal"] = len(set(reports)) == 1
        result["compared_checksums_equal"] = len({r["ordered_checksum"] for r in runs}) == 1
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
