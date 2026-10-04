#!/usr/bin/env python3
"""Compact evidence summary from saved static artifacts."""
import collections
import json
import pathlib
import re

base = pathlib.Path(__file__).parent
final = json.loads((base/'samsung-pass-final.json').read_text())
delta = json.loads(pathlib.Path('/tmp/opencode/samsung-pass-validation/coverage-delta.json').read_text())
tests = pathlib.Path('/tmp/opencode/samsung-pass-workspace-tests.log').read_text()
rows = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', tests)
test_counts = dict(zip(('passed', 'failed', 'ignored'), (sum(int(r[i]) for r in rows) for i in range(3))))
original = [json.loads(line) for line in pathlib.Path(final['method_coverage_jsonl']).read_text().splitlines()]
samsung = [row for row in original if row['identity'].startswith('Lcom/samsung/android/samsungpass/')]
result = {'tests': test_counts, 'test_failures': 'FAILED' in tests,
          'dex_count': len(final['dex']), 'native_libraries': final['native_libraries_unsupported_not_decompiled'], 'assets': final['assets'],
          'identity_status': final['identity_status_counts'], 'source_status': final['source_status_counts'],
          'samsung_original_package_methods': len(samsung), 'samsung_original_package_status': collections.Counter(r['status'] for r in samsung),
          'samsung_original_package_units_written': len(final['original_samsung_package_units']),
          'samsung_gained_identity_samples': [r for r in delta['gained'] if 'Lcom/samsung/android/samsungpass/' in r['identity']][:8],
          'top_categories': {k: v[:7] for k, v in final['category_reasons'].items()},
          'facade_complete_percent': final['complete_bodies_all_original_methods_percent'],
          'explicit_body_percent': final['explicit_body_provenance_all_original_methods_percent'],
          'rejected_all_methods_percent': final['rejected_all_original_methods_percent']}
(base/'samsung-pass-summary.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result, indent=2))
