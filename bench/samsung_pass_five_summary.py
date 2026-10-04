#!/usr/bin/env python3
"""Condense measured ledgers without changing their coverage definitions."""
import json
import collections
import pathlib

base = pathlib.Path(__file__).resolve().parent
r = json.loads((base/'samsung-pass-five-blockers-validation.json').read_text())
before = json.loads((base/'samsung-pass-next-wave-final.json').read_text())
after = json.loads((base/'samsung-pass-five-blockers-final.json').read_text())
delta = json.loads(pathlib.Path(r['validation']['identity_delta']).read_text())
accepted = {'complete_body', 'proven_enum_equivalence'}
category_status = collections.defaultdict(collections.Counter)
category_status_before = collections.defaultdict(collections.Counter)
for row in map(json.loads, (pathlib.Path(before['source_root']).parent/'method-coverage.jsonl').read_text().splitlines()):
    category_status_before[row['category']][row['status']] += 1
for row in map(json.loads, (pathlib.Path(r['source_root'])/'method-coverage.jsonl').read_text().splitlines()):
    category_status[row['category']][row['status']] += 1
lost = [c for c in delta['changed'] if c['before']['status'] in accepted and c['after']['status'] not in accepted]
gained = [c for c in delta['changed'] if c['before']['status'] not in accepted and c['after']['status'] in accepted]
five_patterns = {
    'ctor_writeinv_initializer': ['final-write', 'initialization', 'constructor'],
    'contextual_locals': ['Zero', 'UnknownValue'],
    'dependency_publication': ['required complete method body unavailable', 'required genuine field declaration unavailable'],
    'externalfields': ['external class closure', 'unresolved field metadata', 'external field requirement'],
    'structure_flow': ['OverlappingRegion'],
}
comparison = {}
for category, patterns in five_patterns.items():
    comparison[category] = {}
    for kind in ('ordinary', 'constructor', 'static_initializer'):
        def count(inv):
            return sum(n for reason, n in inv['category_reasons'].get(kind, []) if any(p in reason for p in patterns))
        comparison[category][kind] = {'before': count(before), 'after': count(after)}
summary = dict(strict_counts=r['strict_identity_counts'], strict_gained=len(gained), strict_lost=lost,
    original_category_status=category_status,
    original_category_status_before=category_status_before,
    metric_definitions={
        'legacy_strict': 'Unchanged legacy transitive same-class emitted-body/declaration closure gate; not whole-app/classpath compilation completeness.',
        'semantic': 'Original identities with real emitted bodies, including callers outside legacy closure; enum equivalence stays separate.',
        'unresolved_dependencies': 'Direct same-class method declarations absent from actual emitted bodies or genuine emitted native/abstract declarations. Cross-class/global closure remains incomplete.'},
    direct_same_class_method_declaration_closed_explicit_bodies=r['semantic_explicit_bodies']-r['semantic_bodies_with_unresolved_dependencies'],
    diagnostic_delta_from_latest_38158=r['validation']['diagnostic_count']-38158,
    actual_output_error_reduction_goal_met=False,
    lost_identity_evidence='Targeted original SSA/production resolver regression observed Type("contextual constant constraints: Indeterminate(UnknownValue)"); unsupported numeric Cast consumer role fails closed.',
    semantic_explicit_bodies=r['semantic_explicit_bodies'],
    semantic_gained=len(r['semantic_gained_identities']), semantic_lost=r['semantic_lost_identities'],
    semantic_bodies_outside_strict_dependency_closure=r['semantic_bodies_outside_strict_dependency_closure'],
    semantic_bodies_with_unresolved_dependencies=r['semantic_bodies_with_unresolved_dependencies'],
    diagnostics=r['validation']['diagnostic_count'], syntax=r['validation']['syntax_gate'],
    full_javac=r['validation']['full_compile'], five_first_rejection_pattern_counts=comparison,
    top_remaining={k: v[:8] for k,v in after['category_reasons'].items()},
    source_counts=after['source_status_counts'],
    tests=r['workspace_test_counts'], checks=r['checks'])
(base/'samsung-pass-five-blockers-summary.json').write_text(json.dumps(summary, indent=2)+'\n')
print(json.dumps({k:v for k,v in summary.items() if k != 'checks'}, indent=2))
