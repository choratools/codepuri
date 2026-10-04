#!/usr/bin/env python3
"""Exact-unit proofs and final summary; generated source is never edited."""
import collections
import hashlib
import json
import os
import pathlib
import re
import subprocess
import time

base = pathlib.Path(__file__).parent
out = pathlib.Path('/tmp/opencode/samsung-pass-next-wave')
validation = json.loads((out/'validation.json').read_text())
after = pathlib.Path(validation['source_root'])
before = pathlib.Path('/tmp/opencode/decomphify-java-corpus/run-1790872983292092140-960741/com.samsung.android.samsungpass/sources')
jdk = pathlib.Path('/home/jtjisgod/tools/jdk-17.0.13+11/bin')
android = '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar'
def compile_unit(label, path):
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    command = [str(jdk/'javac'), '-proc:none', '-Xmaxerrs', '2147483647', '-classpath', android,
               '-sourcepath', str(out/'empty-sourcepath'), '-d', str(out/'classes'), str(path)]
    start = time.monotonic()
    with (out/(label+'.log')).open('w') as log:
        result = subprocess.run(command, stdout=log, stderr=log, timeout=120)
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    return dict(command=command, exit_code=result.returncode, seconds=time.monotonic()-start,
                sha256=digest, log=str(out/(label+'.log')),
                diagnostics=dict(collections.Counter(re.findall(r'error: ([^\n]+)', (out/(label+'.log')).read_text()))))

candidates = []
for p in after.rglob('*.java'):
    relative = p.relative_to(after)
    if not str(relative).startswith(('androidx/', 'com/samsung/android/samsungpass/')):
        continue
    old = before/relative
    if old.exists() and old.read_bytes() != p.read_bytes():
        candidates.append((len(p.read_bytes()), str(relative)))
proofs, attempts = [], []
for _, relative in sorted(candidates)[:60]:
    current = compile_unit('candidate-'+str(len(attempts)), after/relative)
    attempts.append(dict(source_unit=relative, after=current))
    if current['exit_code'] == 0:
        previous = compile_unit('exact-before-'+str(len(proofs)), before/relative)
        assert previous['exit_code'] != 0
        proofs.append(dict(source_unit=relative, before=previous, after=current))
        if len(proofs) == 5:
            break
guideline = {label: compile_unit('guideline-'+label, root/'androidx/constraintlayout/widget/Guideline.java')
             for label, root in [('before', before), ('after', after)]}
checks = json.loads((out/'checks.json').read_text())
env = dict(os.environ, PATH=str(jdk)+':'+os.environ['PATH'])
for label, command in [
    ('nested-tests-final', ['cargo', 'test', '-p', 'decomp', '--test', 'java_classpath', 'attested_member_source_names']),
    ('clippy-final', ['cargo', 'clippy', '-p', 'backend-java', '-p', 'decomp', '--all-targets', '--', '-D', 'warnings']),
    ('format-final', ['rustfmt', '--edition', '2024', '--check',
        'crates/decomp/src/java_external_metadata.rs', 'crates/decomp/src/java_classpath.rs',
        'crates/decomp/src/java_aliases.rs', 'crates/decomp/src/java_integration.rs',
        'crates/decomp/tests/java_classpath.rs'])]:
    with (out/(label+'.log')).open('w') as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=log, timeout=180)
    assert result.returncode == 0
    checks[label] = dict(command=command, exit_code=result.returncode, log=str(out/(label+'.log')))
view_command = [str(jdk/'javap'), '-classpath', android, '-public', 'android.view.View']
view = subprocess.run(view_command, capture_output=True, text=True, timeout=30)
assert view.returncode == 0
(out/'view-declarations.log').write_text(view.stdout)
view_constructors = [line.strip() for line in view.stdout.splitlines() if 'public android.view.View(' in line]
assert view_constructors and not any('View()' in line for line in view_constructors)
constructor_identities = [r for r in map(json.loads, (after.parent/'method-coverage.jsonl').read_text().splitlines())
                          if r['identity'].startswith('Landroidx/constraintlayout/widget/Guideline;-><init>')]
text = pathlib.Path(checks['workspace-tests']['log']).read_text()
rows = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
checks['workspace-tests']['counts'] = dict(zip(('passed', 'failed', 'ignored'), (sum(int(r[i]) for r in rows) for i in range(3))))
groups = {label: json.loads(path.read_text()) for label, path in [
    ('before', pathlib.Path('/tmp/opencode/samsung-pass-protected-final/diagnostic-groups.json')),
    ('after', out/'diagnostic-groups.json')]}
prior = json.loads(pathlib.Path('/tmp/opencode/samsung-pass-protected-final/validation.json').read_text())
categories = {}
for label, v in [('before', prior), ('after', validation)]:
    d = v['diagnostics']
    categories[label] = dict(total=sum(d.values()), cannot_find_symbol=d.get('cannot find symbol', 0),
        protected_access=sum(n for k,n in d.items() if 'protected access' in k),
        private_access=sum(n for k,n in d.items() if 'private access' in k),
        incompatible_types=sum(n for k,n in d.items() if 'incompatible types' in k),
        cyclic_inheritance=sum(n for k,n in d.items() if 'cyclic inheritance' in k))
result = dict(validation, checks=checks, diagnostic_grouping=groups, diagnostic_categories=categories,
              exact_unit_proofs=proofs, exact_unit_attempts=attempts, constructor_probe=guideline,
              identity_delta=json.loads((out/'identity-delta.json').read_text()),
              constructor_original_identities=constructor_identities,
              constructor_parent_metadata=dict(command=view_command, constructors=view_constructors, log=str(out/'view-declarations.log')),
              export_source_root=str(after), export_reported_seconds=51.037,
              limits=['All-original identities unchanged; source naming correctness is not a body recovery gain.',
                      'Missing-symbol groups contain unresolved aliases and members; absent libraries are not their sole cause.',
                      'Guideline original constructor family remains rejected: FieldInitializationUnavailable; View has no zero-argument constructor.',
                      'No synthetic constructor, missing API, field initializer, or method body was supplied.',
                      'Compilation diagnostics are observed attribution failures, not a census of every latent defect.'])
(base/'samsung-pass-next-wave-validation.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(dict(categories=categories, compiled_exact_units=[p['source_unit'] for p in proofs],
                     tests=checks['workspace-tests']['counts'], constructor_probe=guideline), indent=2))
