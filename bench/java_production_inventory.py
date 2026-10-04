#!/usr/bin/env python3
"""Audit emitted feature sites and report reasons against a preserved artifact."""
import collections
import json
import pathlib
import re
import sys


def inventory(root):
    methods = {}
    counts = collections.Counter()
    examples = collections.defaultdict(list)
    files = sorted((root / "sources").rglob("*.java"))
    for path in files:
        source = path.read_text()
        relative = str(path.relative_to(root / "sources"))
        features = {
            "while_statements": len(re.findall(r"^\s*while \(true\) \{", source, re.M)),
            "switch_statements": len(re.findall(r"^\s*switch \(", source, re.M)),
            "try_statements": len(re.findall(r"^\s*try \{", source, re.M)),
            "annotation_declarations": len(re.findall(r"^.*@interface ", source, re.M)),
            "annotation_default_elements": len(re.findall(r"^\s*[^/\n]+\(\) default ", source, re.M)),
        }
        active = []
        nested = 0
        for line in source.splitlines():
            indent = len(line) - len(line.lstrip())
            if line.strip() == "}":
                active = [level for level in active if level < indent]
            if line.strip() == "while (true) {":
                nested += bool(active)
                active.append(indent)
        features["nested_while_statements"] = nested
        for name, count in features.items():
            counts[name] += count
            counts[name + "_files"] += bool(count)
            if count and len(examples[name]) < 8:
                examples[name].append({"path": relative, "sites": count})
        for match in re.finditer(r"    // original method pool index (\d+); ([^\n]*); complete_body=(true|false)\n(.*?)(?=    // (?:original method pool index|unavailable original method)|\Z)", source, re.S):
            # Keep identities independent of alias declaration spelling.
            index, key, body, text = match.groups()
            methods[(relative, index)] = {"key": key, "body": body == "true", "source": text}
    counts["java_files"] = len(files)
    return methods, counts, examples


root, old = map(pathlib.Path, sys.argv[1:3])
methods, counts, examples = inventory(root)
previous, baseline_counts, _ = inventory(old)
added = collections.Counter()
added_examples = collections.defaultdict(list)
for identity, method in methods.items():
    if identity in previous:
        continue
    source = method["source"]
    kinds = ["body" if method["body"] else "declaration"]
    if "switch (" in source:
        kinds.append("switch_body")
    if "while (true)" in source:
        kinds.append("while_body")
    if "try {" in source:
        kinds.append("try_body")
    if " default " in source:
        kinds.append("annotation_default_declaration")
    for kind in kinds:
        added[kind] += 1
        if len(added_examples[kind]) < 10:
            added_examples[kind].append({"path": identity[0], "index": identity[1], "key": method["key"]})
reasons = collections.Counter()
typed = collections.Counter()
specific = {}
for line in (root / "report.txt").read_text().splitlines()[1:]:
    tag, count, reason = line.split("\t", 2)
    assert tag == "reason"
    count = int(count)
    reasons[reason.split(":", 1)[0]] += count
    for label, token in {
        "cyclic_control_flow": "CyclicControlFlow",
        "overlapping_region": "OverlappingRegion",
        "unreachable_blocks": "UnreachableBlocks",
        "type_mismatch": "TypeMismatch",
        "unavailable_values": "Unavailable(ValueId",
        "enum_declaration_deferred": "EnumConstructorsUnavailable",
        "branch_unsupported_instruction": "UnsupportedInstruction",
    }.items():
        if token in reason:
            typed[label] += count
    if "exception scope:" in reason or "exception plan:" in reason or "annotation default dependency:" in reason:
        specific[reason] = count
test_log = pathlib.Path(sys.argv[3]).read_text()
test_counts = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", test_log)
print(json.dumps({
    "artifact": str(root), "baseline": str(old), "sites": dict(counts),
    "baseline_sites": dict(baseline_counts), "examples": examples,
    "added_retained_methods": dict(added), "added_examples": added_examples,
    "removed_retained_methods": len(previous.keys() - methods.keys()),
    "residual_reason_categories": dict(reasons), "specific_residuals": specific,
    "typed_residual_filters": dict(typed),
    "tests": {name: sum(int(row[i]) for row in test_counts) for i, name in enumerate(["passed", "failed", "ignored"])},
    "test_warnings": len(re.findall(r"^warning:", test_log, re.M)),
}, indent=2))
