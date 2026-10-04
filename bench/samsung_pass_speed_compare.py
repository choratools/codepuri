#!/usr/bin/env python3
"""Compare complete generated source trees; do not normalize or omit source bytes."""
import hashlib
import json
import pathlib
import re
import sys

before, after, output = map(pathlib.Path, sys.argv[1:4])

def inventory(root):
    return {str(path.relative_to(root / 'sources')): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in (root / 'sources').rglob('*.java')}

def metrics(root):
    lines = (root / 'summary.txt').read_text().splitlines()
    header = dict(re.findall(r'(\w+)=([^ ]+)', lines[0]))
    report = lines[1]
    keys = ['complete_methods', 'complete_constructors', 'complete_static_initializers',
            'declaration_only_methods', 'unsupported_methods', 'unavailable_constructors',
            'classes_seen', 'fields_emitted', 'unavailable_initialization_units']
    return {'header': header, 'counts': {key: int(re.search(r'\b' + key + r': (\d+)', report)[1]) for key in keys},
            'stages': [line for line in lines if line.startswith(('serial\t', 'task_stage\t'))]}

old, new = inventory(before), inventory(after)
left, right = metrics(before), metrics(after)
result = {'before': str(before), 'after': str(after), 'before_metrics': left, 'after_metrics': right,
          'before_files': len(old), 'after_files': len(new),
          'missing': sorted(old.keys() - new.keys()), 'added': sorted(new.keys() - old.keys()),
          'changed': sorted(key for key in old.keys() & new.keys() if old[key] != new[key]),
          'counts_equal': left['counts'] == right['counts']}
result['exact_sources_equal'] = not (result['missing'] or result['added'] or result['changed'])
result['speedup'] = float(left['header']['export_wall_s']) / float(right['header']['export_wall_s'])
output.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({key: result[key] for key in ['before_files', 'after_files', 'counts_equal', 'exact_sources_equal', 'speedup']}))
if not result['counts_equal'] or not result['exact_sources_equal']:
    sys.exit(1)
