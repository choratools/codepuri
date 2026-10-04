#!/usr/bin/env python3
"""Fresh Pass30 source-closure evidence; all sources and diagnostics retained."""
import collections
import hashlib
import json
import os
import pathlib
import re
import subprocess
import time

BASE = pathlib.Path(__file__).resolve().parent
PREFIX = 'samsung-pass-sourceclosure-final'
OUT = pathlib.Path('/tmp/opencode') / PREFIX
OUT.mkdir(exist_ok=True)
JDK = '/home/jtjisgod/tools/jdk-17.0.13+11'
ANDROID = '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar'
ENV = dict(os.environ, JAVA_HOME=JDK, PATH=JDK+'/bin:'+os.environ['PATH'])
ORIGINAL = pathlib.Path('/tmp/opencode/decomphify-java-corpus/run-1790888814662654132-2680151/com.samsung.android.samsungpass')
prior = json.loads((BASE/'samsung-pass-five-final-validation.json').read_text())
PRIOR = pathlib.Path(prior['source_root'])
checks = {}

def save(name, value):
    (BASE/(PREFIX+name+'.json')).write_text(json.dumps(value, indent=2)+'\n')

def run(name, command, timeout=1800):
    start = time.monotonic()
    with (OUT/(name+'.log')).open('w') as log:
        result = subprocess.run(['/usr/bin/time', '-v', '-o', str(OUT/(name+'.time'))]+command,
                                env=ENV, stdout=log, stderr=log, timeout=timeout)
    timing = (OUT/(name+'.time')).read_text()
    item = dict(command=list(map(str, command)), exit_code=result.returncode,
                wall_seconds=time.monotonic()-start,
                max_rss_kib=int(re.search(r'Maximum resident set size \(kbytes\): (\d+)', timing)[1]),
                log=str(OUT/(name+'.log')))
    checks[name] = item
    save('-checks', checks)
    print(name, json.dumps(item), flush=True)
    return result.returncode

for name, command in [('fmt', ['cargo', 'fmt', '--all', '--check']),
                      ('workspace-tests', ['cargo', 'test', '--workspace']),
                      ('clippy', ['cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings']),
                      ('build', ['cargo', 'build', '--release', '-p', 'decomp', '--example', 'java_corpus'])]:
    assert run(name, command) == 0, name
assert run('export', ['target/release/examples/java_corpus', '--workers', '30',
    '--classpath', JDK+'/jmods/java.base.jmod', '--classpath', ANDROID,
    '--app', 'tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk']) == 0
log = (OUT/'export.log').read_text()
match = re.search(r'(/tmp/opencode/decomphify-java-corpus/run-[^\s]+)', log)
assert match, log
root = pathlib.Path(match[1])/'com.samsung.android.samsungpass'
assert run('inventory', ['python3', str(BASE/'samsung_pass_inventory.py'),
    'tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk', str(root), str(BASE/(PREFIX+'-inventory.json'))]) == 0
assert run('validation', ['python3', str(BASE/'samsung_pass_continue.py'), str(ORIGINAL), str(root), str(OUT/'validation')]) == 0
validation = json.loads((OUT/'validation/validation.json').read_text())
assert run('diagnostics', ['python3', str(BASE/'samsung_pass_diagnostics.py'), str(root),
    validation['full_compile']['log'], str(BASE/(PREFIX+'-diagnostics.json'))]) == 0

def ledger(path):
    return {r['identity']: r for r in map(json.loads, (path/'method-coverage.jsonl').read_text().splitlines())}

def identity(line):
    m = re.search(r'MethodKey \{ owner: "([^"]+)", name: "([^"]+)", parameters: (\[[^\]]*\]), return_type: "([^"]+)" \}', line)
    assert m, line
    owner, name, params, result = m.groups()
    return owner+'->'+name+'('+''.join(json.loads(params))+')'+result

def semantic(path):
    ids = {k for k, v in ledger(path).items() if v['status'] == 'complete_body'}
    outside, missing, fields, initialization = set(), set(), set(), set()
    for p in (path/'sources').rglob('*.java'):
        for line in p.read_text().splitlines():
            if '// semantically complete original method pool index ' in line:
                outside.add(identity(line))
            if line.startswith('// Global method source closure:'):
                missing.add(identity(line))
            if line.startswith('// Global field source closure:'):
                fields.add(identity(line))
                if 'InitializationUnavailable' in line:
                    initialization.add(identity(line))
    return ids | outside, outside, missing, fields, initialization

a, p, b = ledger(ORIGINAL), ledger(PRIOR), ledger(root)
assert a.keys() == p.keys() == b.keys()
accepted = {'complete_body', 'proven_enum_equivalence'}
sem, outside, missing, fields, initialization = semantic(root)
old_sem, *_ = semantic(ORIGINAL)
prior_sem, *_ = semantic(PRIOR)
coverage = {'denominator': len(b), 'strict_statuses': collections.Counter(v['status'] for v in b.values()),
            'semantic_explicit_bodies': len(sem), 'outside_strict_sameclass_closure': len(outside),
            'global_direct_missing_method_callers': sorted(missing),
            'global_field_blocked_callers': sorted(fields), 'source_missing_initialization_callers': sorted(initialization)}
for label, old, old_ids in [('original', a, old_sem), ('prior', p, prior_sem)]:
    coverage[label+'_delta'] = {
        'strict_gained': sorted(k for k in b if old[k]['status'] not in accepted and b[k]['status'] in accepted),
        'strict_lost': sorted(k for k in b if old[k]['status'] in accepted and b[k]['status'] not in accepted),
        'semantic_gained': sorted(sem-old_ids), 'semantic_lost': sorted(old_ids-sem)}
save('-coverage', coverage)

def classify(message):
    if message == 'cannot find symbol' or ('package ' in message and ' does not exist' in message) or 'does not override abstract method' in message:
        return 'missing_declaration'
    if 'constructor ' in message and ('cannot be applied' in message or 'no suitable' in message):
        return 'constructor_selection_or_missing_overload'
    return 'code_or_type_error'

def errors(log_path, source_root):
    messages = collections.defaultdict(collections.Counter)
    for path, message in re.findall(r'^(/.*\.java):\d+: error: (.*)$', pathlib.Path(log_path).read_text(), re.M):
        messages[str(pathlib.Path(path).relative_to(source_root))][message] += 1
    return messages

new_errors = errors(validation['full_compile']['log'], root/'sources')
comparisons = {}
baseline_validation = json.loads((BASE/'samsung-pass-next-wave-validation.json').read_text())
for label, old_root, old_log in [('original', ORIGINAL/'sources', baseline_validation['full_compile']['log']),
                                ('prior', PRIOR/'sources', prior['validation']['full_compile']['log'])]:
    old_errors = errors(old_log, old_root)
    old_files = {str(f.relative_to(old_root)) for f in old_root.rglob('*.java')}
    new_files = {str(f.relative_to(root/'sources')) for f in (root/'sources').rglob('*.java')}
    common = old_files & new_files
    rows = {f: {'before': old_errors[f], 'after': new_errors[f], 'in_before': f in old_files, 'in_after': f in new_files}
            for f in sorted(old_files | new_files)}
    groups = {}
    for scope, files in [('common', common), ('old_only', old_files-new_files), ('new_only', new_files-old_files)]:
        groups[scope] = {'files': len(files), 'before': collections.Counter(), 'after': collections.Counter()}
        for f in files:
            for when, source in [('before', old_errors), ('after', new_errors)]:
                for message, n in source[f].items():
                    groups[scope][when][classify(message)] += n
    comparisons[label] = {'groups': groups, 'per_file': rows,
                          'bounds': 'Exact matching source-unit identities; diagnostic classification is not a causal proof for every error.'}
save('-per-file-errors', comparisons)

# Compile unchanged actual original classes; no fabricated dependencies or source edits.
subset = OUT/'original-subsets'
subset.mkdir(exist_ok=True)
units = [root/'sources/com/samsung/android/samsungpass/passservice/mdl'/n for n in
         ('CryptoLibException.java', 'BiometricTokenExpiredException.java', 'InvalidBiometricException.java')]
hashes = {str(f): hashlib.sha256(f.read_bytes()).hexdigest() for f in units}
run('original-three-class-subset', [JDK+'/bin/javac', '-proc:none', '-sourcepath', str(OUT/'validation/empty-sourcepath'), '-d', str(subset)]+list(map(str, units)))
guideline = root/'sources/androidx/constraintlayout/widget/Guideline.java'
hashes[str(guideline)] = hashlib.sha256(guideline.read_bytes()).hexdigest()
run('original-guideline', [JDK+'/bin/javac', '-proc:none', '-classpath', ANDROID, '-sourcepath', str(OUT/'validation/empty-sourcepath'), '-d', str(subset), str(guideline)])
assert hashes == {f: hashlib.sha256(pathlib.Path(f).read_bytes()).hexdigest() for f in hashes}
tests = collections.Counter()
for passed, failed, ignored in re.findall(r'test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;', (OUT/'workspace-tests.log').read_text()):
    tests.update(passed=int(passed), failed=int(failed), ignored=int(ignored))
result = {'source_root': str(root), 'checks': checks, 'workspace_tests': tests, 'validation': validation,
          'coverage': coverage, 'diagnostic_comparison': {k: v['groups'] for k,v in comparisons.items()},
          'original_wholeclass_source_hashes': hashes, 'export_log': log,
          'bounds': 'AnalystAll output. Global declaration availability and initialization evidence are separate from semantic bodies and legacy strict same-class coverage. Whole-app compilation is not asserted.'}
save('', result)
text = '# Samsung Pass source closure — fresh final\n\n'+result['bounds']+'\n\n'
text += '| Metric | Prior final | Source-closure final |\n|---|---:|---:|\n'
text += f"| Strict explicit bodies | 81,828 | {coverage['strict_statuses']['complete_body']:,} |\n| Semantic explicit bodies | 97,199 | {len(sem):,} |\n| Published units | 26,270 | {validation['input_units']:,} |\n| Full uncapped javac errors | 51,302 | {validation['diagnostic_count']:,} |\n"
text += f"\nDefinitions: **{len(b):,}**. Global direct missing-method callers: **{len(missing):,}**. Field-blocked callers: **{len(fields):,}**; initialization-blocked callers: **{len(initialization):,}**. All exact identities and diagnostic messages are saved in the companion JSON files.\n"
text += f"\nTests: **{dict(tests)}**. Syntax gate exit: **{validation['syntax_gate']['exit_code']}**. Full javac exit: **{validation['full_compile']['exit_code']}**; cap reached: **{validation['diagnostic_cap_reached']}**. Sources unchanged: **{validation['source_hashes_unchanged']}**.\n"
text += f"\nPass30 export: **{checks['export']['wall_seconds']:.3f} s / {checks['export']['max_rss_kib']:,} KiB RSS**. Exact JDK17 java.base.jmod and existing Android35 jar. Source root: `{root}`.\n"
(BASE/(PREFIX+'.md')).write_text(text)
print(json.dumps({'strict': coverage['strict_statuses'], 'semantic': len(sem), 'errors': validation['diagnostic_count'], 'tests': tests}, indent=2), flush=True)
