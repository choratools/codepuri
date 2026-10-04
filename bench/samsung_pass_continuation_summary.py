#!/usr/bin/env python3
"""Save compact final continuation evidence, keeping prior-wave artifacts intact."""
import collections
import json
import pathlib
import re

base = pathlib.Path(__file__).parent
final = json.loads((base/'samsung-pass-continuation-final.json').read_text())
out = pathlib.Path('/tmp/opencode/samsung-pass-continuation-final')
validation = json.loads((out/'validation.json').read_text())
delta = json.loads((out/'identity-delta.json').read_text())
sources = pathlib.Path(validation['source_root'])
fields = []
for path in sorted(sources.rglob('*.java')):
    for index, reason in re.findall(r'// unavailable original field index (\d+): ([^\n]+)', path.read_text()):
        fields.append({'source_path': str(path), 'field_index': int(index), 'reason': reason})
log = (out/'workspace-tests.log').read_text()
rows = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', log)
tests = dict(zip(('passed', 'failed', 'ignored'), (sum(int(r[i]) for r in rows) for i in range(3))))
assert 'FAILED' not in log
lost = [r for r in delta['changed'] if r['before']['status'] == 'complete_body' and r['after']['status'] != 'complete_body']
result = {'final_inventory': str(base/'samsung-pass-continuation-final.json'), 'source_root': str(sources), 'syntax_gate': validation['syntax_gate'], 'parse_block_cases': {'CertificateViewModel': 'retained interface subset, field and dependent body explicitly unavailable', 'Lm0/b0;': 'unit unavailable after dependent getKey body rejected; no blank interface field published'}, 'validation_artifact': str(out/'validation.json'), 'full_compile': validation['full_compile'], 'full_diagnostic_count': validation['diagnostic_count'], 'diagnostic_cap_reached': validation['diagnostic_cap_reached'], 'cyclic_inheritance_diagnostics': {k:v for k,v in validation['diagnostics'].items() if 'cyclic inheritance' in k}, 'cannot_find_symbol': validation['diagnostics'].get('cannot find symbol', 0), 'java_xml_module_conflicts': validation['diagnostics'].get('package exists in another module: java.xml', 0), 'protected_access_diagnostics': {k:v for k,v in validation['diagnostics'].items() if 'has protected access' in k}, 'tests': tests, 'identity_status_counts': validation['identity_status_counts'], 'gained_bodies': validation['gained_bodies'], 'lost_bodies': validation['lost_bodies'], 'lost_body_categories': collections.Counter(r['before']['category'] for r in lost), 'lost_identities': lost, 'unavailable_field_declarations_in_written_units': len(fields), 'unavailable_field_reason_counts': collections.Counter(r['reason'] for r in fields), 'unavailable_field_samples': fields[:10], 'representatives': validation['representatives'], 'source_hashes_unchanged': validation['source_hashes_unchanged'], 'export_seconds': 47.504, 'workers': 30, 'metadata': ['java.base.jmod JDK17', 'android-35/android.jar'], 'metadata_cache': 'Existing per-input classpath and per-DEX DexMetadata/Resolver indexes shared by class workers; no per-class rebuild added', 'compiled_apk': False}
(base/'samsung-pass-continuation-validation.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k not in {'representatives', 'lost_identities', 'unavailable_field_samples'}}, indent=2))
