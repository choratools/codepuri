#!/usr/bin/env python3
"""Summarize the fresh final run and retain every old/common-file diagnostic."""
import collections
import json
import pathlib
import re

base = pathlib.Path(__file__).resolve().parent
prefix = 'samsung-pass-five-final'
v = json.loads((base / (prefix + '-validation.json')).read_text())
i = json.loads((base / (prefix + '-inventory.json')).read_text())
d = json.loads((base / (prefix + '-diagnostics.json')).read_text())
old = json.loads((base / 'samsung-pass-next-wave-validation.json').read_text())
old_root = pathlib.Path(old['source_root'])
new_root = pathlib.Path(v['source_root']) / 'sources'

def per_file(log, root):
    counts = collections.Counter()
    messages = collections.defaultdict(collections.Counter)
    for path, message in re.findall(r'^(/.*\.java):\d+: error: (.*)$', pathlib.Path(log).read_text(), re.M):
        relative = str(pathlib.Path(path).relative_to(root))
        counts[relative] += 1
        messages[relative][message] += 1
    return counts, messages

a, am = per_file(old['full_compile']['log'], old_root)
b, bm = per_file(v['validation']['full_compile']['log'], new_root)
old_files = {str(p.relative_to(old_root)) for p in old_root.rglob('*.java')}
new_files = {str(p.relative_to(new_root)) for p in new_root.rglob('*.java')}
common = old_files & new_files
comparison = {
    'baseline_log': old['full_compile']['log'],
    'final_log': v['validation']['full_compile']['log'],
    'common_files': len(common),
    'old_only_files': len(old_files - new_files),
    'new_only_files': len(new_files - old_files),
    'old_common_errors': sum(a[p] for p in common),
    'final_common_errors': sum(b[p] for p in common),
    'old_only_errors': sum(a[p] for p in old_files - new_files),
    'new_only_errors': sum(b[p] for p in new_files - old_files),
    'per_file': {p: {'old': a[p], 'final': b[p], 'delta': b[p] - a[p],
                      'in_old': p in old_files, 'in_final': p in new_files,
                      'old_messages': am[p], 'final_messages': bm[p]}
                 for p in sorted(set(a) | set(b))},
    'interpretation': 'Full published-source compilations; common-file deltas are attribution counts, not matched-body proof. Dependencies and retained bodies can change diagnostics in existing files. No bodies or units filtered.'
}
(base / (prefix + '-per-file-errors.json')).write_text(json.dumps(comparison, indent=2) + '\n')
delta_path = pathlib.Path(v['validation']['identity_delta'])
delta = json.loads(delta_path.read_text())
accepted = {'complete_body', 'proven_enum_equivalence'}
gained = [r['identity'] for r in delta['changed'] if r['before']['status'] not in accepted and r['after']['status'] in accepted]
lost = [r['identity'] for r in delta['changed'] if r['before']['status'] in accepted and r['after']['status'] not in accepted]
restored = 'Ld1/g;->a(I)J'
ledger = {r['identity']: r for r in map(json.loads, (new_root.parent / 'method-coverage.jsonl').read_text().splitlines())}
assert ledger[restored]['status'] == 'complete_body', ledger[restored]
summary = {
    'supersedes': 'samsung-pass-five-blockers final/mid documents',
    'strict_gained': gained, 'strict_lost': lost,
    'restored_identity': ledger[restored], 'metrics': v,
    'diagnostic_categories': d['groups'],
    'per_file_comparison': {k: n for k, n in comparison.items() if k != 'per_file'},
    'bounds': v['integration_bounds']
}
(base / (prefix + '-summary.json')).write_text(json.dumps(summary, indent=2) + '\n')
statuses = v['strict_identity_counts']
rows = [
    ('Strict explicit bodies', 79340, statuses['complete_body']),
    ('Proven enum equivalents', 257, statuses['proven_enum_equivalence']),
    ('Bodyless declarations', 11140, statuses['declaration_only']),
    ('Strict rejects', 114760, statuses.get('rejected_in_written_unit', 0) + statuses.get('rejected_without_written_unit', 0)),
    ('Java files', 24749, v['validation']['input_units']),
    ('Uncapped javac errors', 38158, v['validation']['diagnostic_count']),
]
text = '# Samsung Pass five — fresh final evidence\n\nSupersedes `samsung-pass-five-blockers.md` and its intermediate/final metrics. **Partial integration; cross-DEX source-field publication is still deferred.**\n\n'
text += '| Metric | Original | Fresh final |\n|---|---:|---:|\n'
text += ''.join(f'| {name} | {before:,} | {after:,} |\n' for name, before, after in rows)
text += f"\nImmutable denominator: **205,497** original definitions. Strict body/equivalence identities: **{len(gained):,} gained, {len(lost):,} lost**. `Ld1/g;->a(I)J` is restored and asserted complete in the final ledger.\n"
text += f"\nSemantic explicit bodies: **{v['semantic_explicit_bodies']:,}**; outside strict closure: **{v['semantic_bodies_outside_strict_dependency_closure']:,}**; directly unresolved same-class dependencies: **{v['semantic_bodies_with_unresolved_dependencies']:,}**. Semantic gained/lost against original explicit set: **{len(v['semantic_gained_identities']):,}/{len(v['semantic_lost_identities']):,}**. These metrics remain separate.\n\n## Production bounds\n\n"
for name, bound in v['integration_bounds'].items():
    text += f'- **{name}:** {bound}\n'
text += '\nNumeric Cast consumers resolve the actual instruction target through `JavaResolver::instruction_type`; numeric source categories remain distinct from conversion targets. Synthetic targets are int, long, float, double, byte, char and short. Boolean/void conversion targets reject; unavailable metadata stays indeterminate. Genuine category-one zero materializes as int when hoisted; proven edge coercions select 0/false/null without replacing selected phi identities. Every selected phi and downstream copy receives one atomic concrete declaration. Original SSA and predecessor copy snapshots are retained.\n'
text += '\nInherited fields use actual owner qualification and aliases. Instance receiver casts retain hidden-field selection; protected inherited production fields remain bounded by the existing attestation path. Cross-DEX publication and globally attested explicit initialization are **not implemented**. Existing static ConstantValue source reads retain the prior Java constant-inlining limitation; exact class-initialization preservation is not claimed for that subset.\n'
text += '\n## Validation\n\n'
text += f"Workspace tests: **{v['workspace_test_counts']['passed']} passed, {v['workspace_test_counts']['failed']} failed, {v['workspace_test_counts']['ignored']} ignored**. Full workspace/all-target strict clippy, fmt and release build recorded in final checks. JDK17 runtime phi fixtures cover integer/boolean/reference zero joins, selected branch effects, copies and incompatible arena escapes; original Samsung SSA regression and genuine Android35 inherited-owner source compile pass. Existing protected receiver, hidden-field, staticness and cross-DEX resolver tests remain in the full workspace gate.\n"
text += f"\nAll **{v['validation']['input_units']:,}** exact published sources parsed; parser exit **{v['validation']['syntax_gate']['exit_code']}**. Whole-source javac exit **{v['validation']['full_compile']['exit_code']}**; cap **2,147,483,647**, reached: **{v['validation']['diagnostic_cap_reached']}**. No published bodies excluded.\n\n"
text += '| Attribution category | Errors |\n|---|---:|\n' + ''.join(f'| {name} | {count:,} |\n' for name, count in sorted(d['groups'].items(), key=lambda r: -r[1]))
text += f"\nCommon files: **{len(common):,}**; old common errors **{comparison['old_common_errors']:,}**, final common errors **{comparison['final_common_errors']:,}**. Old-only errors **{comparison['old_only_errors']:,}**; new-only errors **{comparison['new_only_errors']:,}**. Per-file old/final counts and messages are saved in `{prefix}-per-file-errors.json`. This measures emission attribution on overlapping files, not isolated matched-method correctness; expanded bodies and dependencies affect common units too. No aggregate error-reduction claim is implied.\n"
text += f"\nFinal source root: `{v['source_root']}`. Exact original three-class JDK17 and Guideline/Android35 subset outcomes and hashes are retained in `{prefix}-validation.json`. All five category/reason counts are in `{prefix}-inventory.json`; strict and semantic exact identities in `{prefix}-summary.json`. Export time/RSS: **{v['checks']['export']['wall_seconds']:.3f} s / {v['checks']['export']['max_rss_kib']:,} KiB**. Final logs: `/tmp/opencode/{prefix}/`.\n"
(base / (prefix + '.md')).write_text(text)
print(json.dumps({'rows': rows, 'strict_gained': len(gained), 'strict_lost': len(lost), 'common_errors': {k: n for k, n in comparison.items() if k.endswith('_errors')}}, indent=2))
