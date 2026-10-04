#!/usr/bin/env python3
"""Bounded category histograms and original method identity samples; no source edits."""
import collections
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
rules = collections.defaultdict(collections.Counter)
totals = collections.Counter()
normalize = lambda text: re.sub(r'(?:ValueId|InsnId|BlockId|EdgeId)\(\d+v\d+\)', 'ID', re.sub(r'MethodKey \{.*?\}', 'METHOD', text))
for line in (root / 'report.txt').read_text().splitlines():
    if line.startswith('category_reason\t'):
        _, count, category, reason = line.split('\t', 3)
        rules[category][normalize(reason)] += int(count)
        totals[category] += int(count)
samples = collections.defaultdict(list)
files = 0
unavailable_initialization = 0
for path in (root / 'sources').rglob('*.java'):
    files += 1
    text = path.read_text()
    unavailable_initialization += '// Original field initialization proof: unavailable.' in text
    for line in text.splitlines():
        if '// unavailable original method index ' not in line:
            continue
        category = 'constructor' if 'name: "<init>"' in line else 'static_initializer' if 'name: "<clinit>"' in line else 'ordinary'
        if len(samples[category]) < 5:
            samples[category].append({'unit': str(path.relative_to(root / 'sources')), 'identity_and_rejection': line.strip()})
tests = None
if len(sys.argv) > 2:
    rows = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', pathlib.Path(sys.argv[2]).read_text())
    tests = dict(zip(('passed', 'failed', 'ignored'), (sum(int(row[i]) for row in rows) for i in range(3))))
print(json.dumps({'workspace_tests': tests, 'source_files': files, 'unavailable_initialization_units': unavailable_initialization,
                  'category_totals': dict(totals), 'top_recorded_rules': {k: v.most_common(12) for k, v in rules.items()},
                  'identity_samples': dict(samples), 'scope': 'Recorded rejection causes, not an enumeration of all latent defects.'}, indent=2))
