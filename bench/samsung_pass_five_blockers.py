#!/usr/bin/env python3
"""Fresh production gates, Pass30 export and separate exact-identity ledgers."""
import collections
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time

BASE = pathlib.Path(__file__).resolve().parent
PREFIX = 'samsung-pass-five-final' if '--final' in sys.argv else 'samsung-pass-five-blockers'
OUT = pathlib.Path('/tmp/opencode')/PREFIX
OUT.mkdir(exist_ok=True)
JDK = '/home/jtjisgod/tools/jdk-17.0.13+11'
ENV = dict(os.environ, JAVA_HOME=JDK, PATH=JDK+'/bin:'+os.environ['PATH'])
BEFORE = pathlib.Path('/tmp/opencode/decomphify-java-corpus/run-1790888814662654132-2680151/com.samsung.android.samsungpass')
checks = {}
RESUME = '--resume' in sys.argv
if RESUME:
    checks = json.loads((BASE/(PREFIX+'-checks.json')).read_text())

def run(name, command, timeout=1800):
    if RESUME and name in checks and checks[name]['exit_code'] == 0 and name in ('workspace-tests',):
        return 0
    start = time.monotonic()
    with (OUT/(name+'.log')).open('w') as log:
        result = subprocess.run(['/usr/bin/time', '-v', '-o', str(OUT/(name+'.time'))]+command,
                                env=ENV, stdout=log, stderr=log, timeout=timeout)
    timing = (OUT/(name+'.time')).read_text()
    item = dict(command=command, exit_code=result.returncode, wall_seconds=time.monotonic()-start,
                max_rss_kib=int(re.search(r'Maximum resident set size \(kbytes\): (\d+)', timing)[1]),
                log=str(OUT/(name+'.log')))
    checks[name] = item
    (BASE/(PREFIX+'-checks.json')).write_text(json.dumps(checks, indent=2)+'\n')
    print(name, json.dumps(item), flush=True)
    return result.returncode

assert run('fmt', ['cargo', 'fmt', '--all', '--check']) == 0
assert run('workspace-tests', ['cargo', 'test', '--workspace']) == 0
assert run('clippy', ['cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings']) == 0
assert run('build', ['cargo', 'build', '--release', '-p', 'decomp', '--example', 'java_corpus']) == 0
if '--checks-only' in sys.argv:
    previous = json.loads((BASE/(PREFIX+'-validation.json')).read_text())
    previous['checks'].update(checks)
    totals = collections.Counter()
    for passed, failed, ignored in re.findall(r'test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;', (OUT/'workspace-tests.log').read_text()):
        totals.update(passed=int(passed), failed=int(failed), ignored=int(ignored))
    previous['workspace_test_counts'] = totals
    (BASE/(PREFIX+'-validation.json')).write_text(json.dumps(previous, indent=2)+'\n')
    sys.exit(0)
assert run('export', ['target/release/examples/java_corpus', '--workers', '30',
    '--classpath', JDK+'/jmods/java.base.jmod', '--classpath', '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar',
    '--app', 'tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk']) == 0
log = (OUT/'export.log').read_text()
print(log, flush=True)
match = re.search(r'(/tmp/opencode/decomphify-java-corpus/run-[^\s]+)', log)
assert match, 'export root missing'
root = pathlib.Path(match[1])/'com.samsung.android.samsungpass'
assert run('inventory', ['python3', str(BASE/'samsung_pass_inventory.py'),
    'tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk', str(root),
    str(BASE/(PREFIX+'-inventory.json'))]) == 0
assert run('validation', ['python3', str(BASE/'samsung_pass_continue.py'), str(BEFORE), str(root), str(OUT/'validation')]) == 0
assert run('diagnostics', ['python3', str(BASE/'samsung_pass_diagnostics.py'), str(root), str(OUT/'validation/full-javac.log'), str(BASE/(PREFIX+'-diagnostics.json'))]) == 0
units = [root/'sources/com/samsung/android/samsungpass/passservice/mdl'/name for name in
         ('CryptoLibException.java', 'BiometricTokenExpiredException.java', 'InvalidBiometricException.java')]
hashes = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in units}
subset_classes = OUT/'original-subset-classes'
subset_classes.mkdir(exist_ok=True)
assert run('jdk17-original-three-class-subset', [JDK+'/bin/javac', '-proc:none', '-sourcepath', str(OUT/'validation/empty-sourcepath'), '-d', str(subset_classes)]+list(map(str, units))) == 0
guideline = root/'sources/androidx/constraintlayout/widget/Guideline.java'
hashes[str(guideline)] = hashlib.sha256(guideline.read_bytes()).hexdigest()
run('original-guideline', [JDK+'/bin/javac', '-proc:none', '-classpath', '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar', '-sourcepath', str(OUT/'validation/empty-sourcepath'), '-d', str(subset_classes), str(guideline)])
assert hashes == {p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in hashes}
inventory = json.loads((BASE/(PREFIX+'-inventory.json')).read_text())
validation = json.loads((OUT/'validation/validation.json').read_text())
def ledger(path):
    return {r['identity']: r for r in map(json.loads, (path/'method-coverage.jsonl').read_text().splitlines())}
old, new = ledger(BEFORE), ledger(root)
assert old.keys() == new.keys()
semantic = {k for k, v in new.items() if v['status'] == 'complete_body'}
unresolved = set()
outside_strict = set()
for source in (root/'sources').rglob('*.java'):
    for line in source.read_text().splitlines():
        if '// semantically complete original method pool index ' not in line:
            continue
        m = re.search(r'MethodKey \{ owner: "([^"]+)", name: "([^"]+)", parameters: (\[[^\]]*\]), return_type: "([^"]+)" \}', line)
        assert m, line
        owner, name, params, result = m.groups()
        identity = owner+'->'+name+'('+''.join(json.loads(params))+')'+result
        assert identity in new and new[identity]['has_code'] and new[identity]['category'] == 'ordinary'
        outside_strict.add(identity)
        if 'dependency_complete=false' in line:
            unresolved.add(identity)
semantic.update(outside_strict)
old_semantic = {k for k, v in old.items() if v['status'] == 'complete_body'}
tests = (OUT/'workspace-tests.log').read_text()
test_counts = collections.Counter()
for passed, failed, ignored in re.findall(r'test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;', tests):
    test_counts.update(passed=int(passed), failed=int(failed), ignored=int(ignored))
result = dict(baseline=dict(explicit_bodies=79340, enum_equivalents=257, rejected=114760, files=24749, diagnostics=38158),
    source_root=str(root), checks=checks, workspace_test_counts=test_counts,
    exact_original_source_hashes=hashes,
    validation=validation, strict_identity_counts=inventory['identity_status_counts'],
    semantic_explicit_bodies=len(semantic), semantic_bodies_with_unresolved_dependencies=len(unresolved),
    semantic_bodies_outside_strict_dependency_closure=len(outside_strict),
    semantic_gained_identities=sorted(semantic-old_semantic), semantic_lost_identities=sorted(old_semantic-semantic),
    semantic_unresolved_identities=sorted(unresolved),
    category_reasons=inventory['category_reasons'],
    integration_bounds={
        'ctor_writeinv_initializer': 'Production initializer inventory remains atomic; direct-parent external constructor-body invoke-super hook integrated.',
        'contextual_locals': 'Atomic checked_phi_plan family declarations active in ordinary structured/switch/shared-forward paths; whole-arena consumers and original reaching inputs checked. Edge sources snapshot before assignments, zero coercions preserve availability. Loop/exception family adoption remains bounded.',
        'dependency_publication': 'Ordinary semantic callers retained separately from legacy strict dependency-closure metric; exact method_closure diagnostics active. Constructor/clinit closure stays atomic.',
        'externalfields': 'Public inherited classpath fields use descriptor-exact actual declaring owner and genuine metadata, original caller/receiver access checks and actual-owner aliases. Input-owned/cross-DEX field publication remains deferred; protected inherited production fields remain fail-closed.',
        'structure_flow': 'Canonical shared acyclic regions active in production with path-availability intersection, edge copies and typed terminal rendering.'})
(BASE/(PREFIX+'-validation.json')).write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps({k: v for k, v in result.items() if k in ('source_root', 'workspace_test_counts', 'strict_identity_counts', 'semantic_explicit_bodies', 'semantic_bodies_with_unresolved_dependencies')}, indent=2))
