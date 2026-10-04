#!/usr/bin/env python3
"""Count actual final Java syntax and normalized first-failing proof rules."""
import collections
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
counts = collections.Counter()
examples = collections.defaultdict(list)
patterns = {
    "enums": r"^\s*(?:public )?enum \w+",
    "labeled_loops": r"^\s*loop\w+: while \(true\)",
    "labeled_breaks": r"^\s*break loop\w+;",
    "labeled_continues": r"^\s*continue loop\w+;",
    "try": r"^\s*try \{",
    "finally": r"\bfinally \{",
    "synchronized": r"^\s*synchronized \(",
    "explicit_super": r"^\s*super\(",
    "explicit_this": r"^\s*this\(",
}
for path in (root / "sources").rglob("*.java"):
    source = path.read_text()
    for name, pattern in patterns.items():
        n = len(re.findall(pattern, source, re.M))
        counts[name] += n
        if n and len(examples[name]) < 5:
            examples[name].append(str(path.relative_to(root / "sources")))
rules = collections.Counter()
for line in (root / "report.txt").read_text().splitlines():
    if not line.startswith("reason\t"):
        continue
    _, n, reason = line.split("\t", 2)
    normalized = re.sub(r"(?:ValueId|InsnId|BlockId|EdgeId)\(\d+v\d+\)", "ID", reason)
    normalized = re.sub(r"MethodKey \{.*?\}", "METHOD", normalized)
    normalized = re.sub(r"InvalidReference\(\d+\)", "InvalidReference(ID)", normalized)
    normalized = re.sub(r"CyclicControlFlow\(\[.*?\]\)", "CyclicControlFlow(BLOCKS)", normalized)
    rules[normalized] += int(n)
parity = None
if len(sys.argv) == 4:
    def normalize_cases(source):
        return re.sub(r"(?:^\s*case -?\d+:\n)+",
                      lambda match: "".join(sorted(match.group().splitlines(keepends=True))),
                      source, flags=re.M)
    baseline, fixture = (pathlib.Path(p).read_text() for p in sys.argv[2:])
    parity = {"byte_equal": baseline == fixture,
              "equal_after_grouped_case_order_normalization": normalize_cases(baseline) == normalize_cases(fixture)}
    assert parity["equal_after_grouped_case_order_normalization"], "default fixture changed beyond grouped-label ordering"
print(json.dumps({"syntax": dict(counts), "examples": dict(examples), "no_classpath_parity": parity,
                  "top_rules": rules.most_common(25),
                  "control_rules": {k: v for k, v in sorted(rules.items())
                      if any(p in k for p in ("exception", "structure:", "branch plan", "constructor hierarchy", "EnumDeclaration"))}}, indent=2))
