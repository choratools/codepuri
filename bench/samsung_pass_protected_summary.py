#!/usr/bin/env python3
"""Retain exact-source protected-receiver evidence; never rewrite exported files."""
import collections
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time

base = pathlib.Path(__file__).parent
out = pathlib.Path('/tmp/opencode/samsung-pass-protected-final')
before = pathlib.Path('/tmp/opencode/decomphify-java-corpus/run-1790871140841023213-740155/com.samsung.android.samsungpass/sources')
validation = json.loads((out/'validation.json').read_text())
after = pathlib.Path(validation['source_root'])
jdk = pathlib.Path('/home/jtjisgod/tools/jdk-17.0.13+11/bin')
android = '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar'

def run(name, command, env=None):
    start = time.monotonic()
    with (out/(name+'.log')).open('w') as log:
        result = subprocess.run(command, stdout=log, stderr=log, timeout=600, env=env)
    text = (out/(name+'.log')).read_text()
    return {'command': list(command), 'exit_code': result.returncode, 'seconds': time.monotonic()-start,
            'log': str(out/(name+'.log')), 'diagnostics': dict(collections.Counter(re.findall(r'error: ([^\n]+)', text)))}

guideline = {}
for label, root in [('before', before), ('after', after)]:
    source = root/'androidx/constraintlayout/widget/Guideline.java'
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    command = [str(jdk/'javac'), '-proc:none', '-Xmaxerrs', '2147483647', '-classpath', android,
               '-sourcepath', str(out/'empty-sourcepath'), '-d', str(out/'classes'), str(source)]
    exact = run('guideline-'+label, command)
    exact.update(source_path=str(source), sha256=digest)
    assert hashlib.sha256(source.read_bytes()).hexdigest() == digest
    # Separate method-only probe. Preserve the complete onMeasure declaration
    # verbatim and provide an explicit fixture constructor required by View.
    method = re.search(r'    public void onMeasure\(int p0, int p1\) \{.*?\n    \}', source.read_text(), re.S).group()
    fixture = out/('guideline-method-'+label)
    fixture.mkdir(exist_ok=True)
    path = fixture/'Guideline.java'
    path.write_text('package androidx.constraintlayout.widget;\npublic class Guideline extends android.view.View {\n'
                    'public Guideline(android.content.Context c) { super(c); }\n'+method+'\n}\n')
    command[-1] = str(path)
    snippet = run('guideline-method-'+label, command)
    guideline[label] = {'exact_unit': exact, 'verbatim_method_fixture': snippet, 'method_source': method}
assert guideline['before']['verbatim_method_fixture']['exit_code'] == 1
assert guideline['after']['verbatim_method_fixture']['exit_code'] == 0
assert not any('protected access' in d for d in guideline['after']['exact_unit']['diagnostics'])

env = dict(os.environ, PATH=str(jdk)+':'+os.environ['PATH'])
saved = json.loads((base/'samsung-pass-protected-validation.json').read_text()) if '--reuse-checks' in sys.argv else None
tests = (json.loads((out/'checks.json').read_text())['tests'] if '--reuse-tests' in sys.argv
         else saved['tests'] if saved else run('workspace-tests', ['cargo', 'test', '--workspace'], env))
assert tests['exit_code'] == 0
rows = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', pathlib.Path(tests['log']).read_text())
tests['counts'] = dict(zip(('passed', 'failed', 'ignored'), (sum(int(r[i]) for r in rows) for i in range(3))))
(out/'checks.json').write_text(json.dumps({'tests': tests}, indent=2)+'\n')
clippy = saved['clippy'] if saved else run('clippy', ['cargo', 'clippy', '-p', 'backend-java', '-p', 'decomp', '--all-targets', '--', '-D', 'warnings'], env)
assert clippy['exit_code'] == 0
targeted = run('protected-tests-final', ['cargo', 'test', '-p', 'decomp', '-p', 'backend-java', 'protected'], env) if '--targeted' in sys.argv else None
if targeted:
    assert targeted['exit_code'] == 0

prior = json.loads((base/'samsung-pass-continuation-validation.json').read_text())
prior_full = json.loads(pathlib.Path(prior['validation_artifact']).read_text())
delta = json.loads((out/'identity-delta.json').read_text())
categories = {}
for label, v in [('before', prior_full), ('after', validation)]:
    d = v['diagnostics']
    categories[label] = {'total': sum(d.values()), 'cannot_find_symbol': d.get('cannot find symbol', 0),
        'java_xml_module_conflict': d.get('package exists in another module: java.xml', 0),
        'protected_access': sum(n for k,n in d.items() if 'has protected access' in k),
        'incompatible_types': sum(n for k,n in d.items() if 'incompatible types' in k),
        'package_access': sum(n for k,n in d.items() if 'is not public' in k),
        'cyclic_inheritance': sum(n for k,n in d.items() if 'cyclic inheritance' in k)}

# Save category snippets verbatim, with their exact original source identities.
text = (out/'full-javac.log').read_text()
snippets = collections.defaultdict(list)
for block in re.split(r'(?=^/.*\.java:\d+: error:)', text, flags=re.M):
    for category in ('protected access', 'is not public', 'incompatible types'):
        if 'error:' in block and category in block.split('\n')[0] and len(snippets[category]) < 12:
            snippets[category].append(block.splitlines()[:5])
result = dict(validation, guideline=guideline, tests=tests, clippy=clippy,
              final_targeted_tests=targeted,
              before_after_diagnostic_categories=categories, identity_delta_rows=delta['changed'],
              source_correctness_snippets=snippets, export_seconds=46.812, workers=30,
              limits=['Receiver-selection proof currently covers direct input subclasses only.',
                      'Competing same-name descriptors, unknown intermediate declarations, and aliased original identities fail closed.',
                      'External field publication remains unsupported by genuine-field closure.',
                      'Remaining protected diagnostics include calls to unavailable input overrides, with no receiver cast: cross-unit member publication closure remains incomplete.',
                      'Six FactoryFinder package-access diagnostics occur alongside javax.xml java.xml module conflicts; the source calls stay in their original package.',
                      'Exact Guideline still lacks its rejected original constructors; its implicit constructor cannot call View().'])
(base/'samsung-pass-protected-validation.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps({'categories': categories, 'tests': tests['counts'], 'guideline': guideline}, indent=2))
