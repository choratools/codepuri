"""Extract exact emitted constructor rejection evidence, without inferring causes."""
import argparse
import collections
import json
import pathlib
import re


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    by_unit = collections.defaultdict(dict)
    for line in (args.root / "method-coverage.jsonl").open():
        row = json.loads(line)
        if row.get("source_unit") and row["category"] == "constructor":
            by_unit[row["source_unit"]][row["method_index"]] = row
    pattern = re.compile(r"^\s*// constructor subset rejection index (\d+): (.*)$")
    records = []
    unmatched = []
    for unit, methods in sorted(by_unit.items()):
        path = args.root / "sources" / unit
        if not path.is_file():
            raise RuntimeError(f"ledger source missing: {path}")
        for number, line in enumerate(path.open(), 1):
            match = pattern.match(line)
            if not match:
                continue
            index = int(match[1])
            row = methods.get(index)
            if row is None:
                unmatched.append({"source_unit": unit, "line": number, "raw": line.rstrip()})
                continue
            evidence = match[2]
            markers = [
                "constructor local evidence:",
                "constructor reachable-chain rejection:",
                "constructor subset: exceptional chain unavailable",
                "constructor subset frontend:",
                "constructor subset proof:",
                "constructor subset: original body not retained",
            ]
            category = next((m for m in markers if m in evidence), "other recorded diagnostic")
            records.append({**row, "source_line": number, "diagnostic_category": category,
                            "diagnostic": evidence})
    result = {"root": str(args.root), "record_count": len(records),
              "categories": dict(collections.Counter(r["diagnostic_category"] for r in records)),
              "records": records, "unmatched": unmatched,
              "limits": "Only exact emitted constructor comments; absent units and unrecorded blockers are not inferred."}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("record_count", "categories")}))
    if unmatched:
        raise RuntimeError(f"{len(unmatched)} diagnostics lacked exact ledger identities")


if __name__ == "__main__":
    main()
