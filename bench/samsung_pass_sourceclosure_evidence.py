#!/usr/bin/env python3
"""Unchanged real wholeclass probes and reconciled final evidence summary."""
import collections
import hashlib
import json
import pathlib
import re
import subprocess

BASE = pathlib.Path(__file__).resolve().parent
PREFIX = 'samsung-pass-sourceclosure-final'
result = json.loads((BASE/(PREFIX+'.json')).read_text())
prior = json.loads((BASE/'samsung-pass-five-final-validation.json').read_text())
comparison = json.loads((BASE/(PREFIX+'-per-file-errors.json')).read_text())
root = pathlib.Path(result['source_root'])
before = pathlib.Path(prior['source_root'])
out = pathlib.Path('/tmp/opencode')/(PREFIX+'-wholeclasses')
out.mkdir(exist_ok=True)
jdk = '/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac'
android = '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar'
empty = pathlib.Path('/tmp/opencode')/PREFIX/'validation/empty-sourcepath'
ledger = {r['identity']: r for r in map(json.loads, (root/'method-coverage.jsonl').read_text().splitlines())}
old = {r['identity']: r for r in map(json.loads, (before/'method-coverage.jsonl').read_text().splitlines())}
recovered = collections.defaultdict(list)
for key in result['coverage']['prior_delta']['strict_gained']:
    if ledger[key]['category'] == 'constructor':
        recovered[ledger[key]['source_unit']].append({'identity': key, 'before': old[key], 'after': ledger[key]})
rows = comparison['prior']['per_file']
aliases = {str(p.relative_to(root/'sources'))[:-5].replace('/', '.') for p in (root/'sources').rglob('*.java')}
dependency_cache = {}
def dependencies(relative):
    if relative in dependency_cache:
        return dependency_cache[relative]
    code = re.sub(r'//[^\n]*', '', (root/'sources'/relative).read_text())
    own = relative[:-5].replace('/', '.')
    found = set()
    for token in re.findall(r'[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)+', code):
        while '.' in token:
            if token in aliases:
                if token != own:
                    found.add(token)
                break
            token = token.rsplit('.', 1)[0]
    dependency_cache[relative] = sorted(found)
    return dependency_cache[relative]
fixed = sorted((p for p,v in rows.items() if sum(v['before'].values()) > 0 and not v['after'] and v['in_after']), key=lambda p: (len(dependencies(p)), p))
ctor_candidates = sorted((p for p in recovered if not rows[p]['after']), key=lambda p: (len(dependencies(p)), p))
probes = []

def probe(relative, category, closure=None):
    closure = closure or [relative]
    item = {'source_unit': relative, 'actual_source_closure': closure, 'category': category, 'recovered_constructors': recovered.get(relative, []), 'runs': {}}
    for when, source_root in [('before', before), ('after', root)]:
        source = source_root/'sources'/relative
        if not source.exists():
            item['runs'][when] = {'source_absent': True}
            continue
        paths = [source_root/'sources'/p for p in closure if (source_root/'sources'/p).exists()]
        hashes = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
        digest = hashes[str(source)]
        dest = out/(str(len(probes))+'-'+when)
        dest.mkdir(exist_ok=True)
        command = [jdk, '-proc:none', '-Xmaxerrs', '2147483647', '-classpath', android,
                   '-sourcepath', str(empty), '-d', str(dest)]+list(map(str, paths))
        run = subprocess.run(command, capture_output=True, timeout=120)
        log = out/(str(len(probes))+'-'+when+'.log')
        log.write_bytes(run.stdout+run.stderr)
        assert hashes == {p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in hashes}
        item['runs'][when] = {'command': command, 'exit_code': run.returncode, 'log': str(log),
                              'sha256': digest, 'source_hashes': hashes, 'source_unchanged': True}
    probes.append(item)
    print(category, relative, {k:v.get('exit_code') for k,v in item['runs'].items()}, flush=True)

# The candidate inventory is complete. These deliberately bounded unchanged
# wholeclass probes supplement the uncapped ALL-source compiler gate.
for relative in fixed[:30]:
    probe(relative, 'previously_failing_retained_unit')
for relative in ctor_candidates[:30]:
    if relative not in {p['source_unit'] for p in probes}:
        probe(relative, 'new_original_constructor_wholeclass')
def source_closure(relative):
    seen, pending = set(), [relative]
    while pending and len(seen) <= 30:
        unit = pending.pop()
        if unit in seen:
            continue
        seen.add(unit)
        pending.extend(d.replace('.', '/')+'.java' for d in dependencies(unit))
    return sorted(seen) if not pending and len(seen) <= 30 else None
closed = [(p, source_closure(p)) for p in fixed]
closed = sorted(((p, c) for p,c in closed if c is not None and len(c) > 1), key=lambda row: (len(row[1]), row[0]))
for relative, closure in closed[:30]:
    probe(relative, 'previously_failing_genuine_source_closure', closure)
evidence = {'fixed_unit_candidates': fixed, 'constructor_candidates': ctor_candidates,
            'bounded_genuine_source_closure_candidates': closed,
            'all_recovered_original_constructors': dict(recovered), 'probes': probes,
            'classpaths': 'Exact JDK17 compiler and existing Android35 jar; empty sourcepath; no source edits, source stubs or invented parent classes.'}
(BASE/(PREFIX+'-wholeclasses.json')).write_text(json.dumps(evidence, indent=2)+'\n')
summary = {'coverage': {k: v for k,v in result['coverage'].items() if not isinstance(v, list) and not k.endswith('_delta')},
           'deltas': {name: {k: len(v) for k,v in result['coverage'][name+'_delta'].items()} for name in ['original', 'prior']},
           'diagnostic_comparison': result['diagnostic_comparison'],
           'fixed_wholeclass_successes': [p['source_unit'] for p in probes if p['runs']['before'].get('exit_code') == 1 and p['runs']['after'].get('exit_code') == 0],
           'recovered_constructor_wholeclass_successes': [p['source_unit'] for p in probes if p['recovered_constructors'] and p['runs']['after'].get('exit_code') == 0],
           'recovered_constructor_count': sum(map(len, recovered.values())),
           'global_direct_missing_method_callers': len(result['coverage']['global_direct_missing_method_callers']),
           'global_field_blocked_callers': len(result['coverage']['global_field_blocked_callers']),
           'source_missing_initialization_callers': len(result['coverage']['source_missing_initialization_callers'])}
source_initialization_missing = []
for path in (root/'sources').rglob('*.java'):
    if '// Original field initialization proof: unavailable.' in path.read_text():
        source_initialization_missing.append(str(path.relative_to(root/'sources')))
summary['source_missing_initialization_units'] = len(source_initialization_missing)
evidence['source_missing_initialization_units'] = source_initialization_missing
(BASE/(PREFIX+'-wholeclasses.json')).write_text(json.dumps(evidence, indent=2)+'\n')
(BASE/(PREFIX+'-summary.json')).write_text(json.dumps(summary, indent=2)+'\n')
doc = BASE/(PREFIX+'.md')
text = doc.read_text().split('\n## Exact identity deltas and diagnostic reconciliation')[0]+'\n## Exact identity deltas and diagnostic reconciliation\n\n'
for label, delta in summary['deltas'].items():
    text += f"- Against **{label}**: strict **{delta['strict_gained']:,} gained / {delta['strict_lost']:,} lost**; semantic **{delta['semantic_gained']:,} gained / {delta['semantic_lost']:,} lost**.\n"
text += '\nThe strict metric remains legacy same-class closure; global source availability is reported separately. AnalystAll retains every previously exported unit.\n\n'
for label, groups in summary['diagnostic_comparison'].items():
    text += f'### {label} source-unit identity comparison\n\n| Scope | Files | Before errors | After errors | After missing declarations | After code/type | After uncertain constructor selection |\n|---|---:|---:|---:|---:|---:|---:|\n'
    for scope, values in groups.items():
        text += f"| {scope} | {values['files']:,} | {sum(values['before'].values()):,} | {sum(values['after'].values()):,} | {values['after'].get('missing_declaration',0):,} | {values['after'].get('code_or_type_error',0):,} | {values['after'].get('constructor_selection_or_missing_overload',0):,} |\n"
text += '\nThese are complete compiler-attribution messages on matched units, not a claim that every classified error has the same root cause. Constructor-selection diagnostics remain explicitly ambiguous.\n'
text += f"\nOriginal constructors newly recovered against prior: **{summary['recovered_constructor_count']:,}**. Initialization-unavailable source units: **{summary['source_missing_initialization_units']:,}**. Descriptor arity is preserved even for unused parameters, without inventing SSA values. The complete constructor identity inventory and original rejection reasons are in `samsung-pass-sourceclosure-final-wholeclasses.json`.\n"
text += '\n## Unchanged real wholeclass evidence\n\n'+evidence['classpaths']+'\n\n'
text += 'Previously failing retained units that now compile with the recorded genuine source closure:\n'+''.join(f'- `{p}`\n' for p in summary['fixed_wholeclass_successes'])
text += '\nWholeclasses with newly recovered original constructors that compile, individually or with the recorded genuine source closure:\n'+''.join(f'- `{p}`\n' for p in summary['recovered_constructor_wholeclass_successes'])
text += '\nAll selected probe failures, exact compiler commands, before/after source hashes and complete logs remain recorded; successful probes do not imply whole-app compilation. The final whole-source gate still fails.\n'
text += '\n## Production integration and remaining bounds\n\n'
text += '- `FieldPublication` is collected before alias freezing with the export’s one write inventory. Actual owner declarations precede method bodies, including field-only owner units. Proven encoded prefixes use frozen names and precede original clinit. Interface fields remain unavailable without a proven legal initializer. Malformed metadata is separately reported.\n'
text += '- `GlobalDeclarations` caches defining-pool locations, flags, exceptions, signatures and body status. The global worklist consumes actual cached semantic publications, visits each original identity once, and reports missing bodies/declarations without creating them. Class limits cannot attest undelivered worker output. Original native/abstract declarations require actual source publication; genuine external declarations require exact classpath metadata.\n'
text += '- Constructor and static branch-tree implementations remain the backend authority; atomic constructor family/hierarchy/super-chain and original-final-field proofs remain required. The family API accepts signature-preserving declaration bindings. `KCDSAPrivateKeySpec` retains its original six-argument constructor and unused `p4`, actual `super()` and actual field writes; the unchanged wholeclass compiles. `CollapsingToolbarLayout$d` retains all three original constructors against the genuine Android `FrameLayout.LayoutParams` parent and compiles with its real emitted `ub/m$type.java` resource-owner unit.\n'
text += '- Analyst output is the default and preserves all old source identities. Optional `StrictValidClasses` remains an explicit helper selection policy; it is not applied to the default export. Strict coverage in this benchmark means the historical same-class body closure, not wholeclass javac validity.\n'
text += '- Source closure is still partial: unavailable method bodies, superclass/interface obligations, unresolved dependencies and incomplete initialization remain visible. Declaration publication does not establish execution-equivalent initialization or full application correctness. The output retains SourceFallback provenance.\n'
text += '- Performance: one cached original-owner index avoids repeated full-input class scans for each field access. Owner field and write lookups are bounded ranges. Pass30 is unchanged; memory grows with the cached global catalog and buffered actual publications.\n'
doc.write_text(text)
print(json.dumps(summary, indent=2))
