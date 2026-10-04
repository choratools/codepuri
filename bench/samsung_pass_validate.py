#!/usr/bin/env python3
"""Compile unmodified emitted units only; fixtures and app execution are excluded."""
import collections
import json
import pathlib
import re
import subprocess
import sys
import time

before, after, out = map(pathlib.Path, sys.argv[1:4])
out.mkdir(exist_ok=True)
jdk = pathlib.Path('/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac')
android = '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar'
pattern = re.compile(r'// original method pool index (\d+); (.*?); complete_body=true')
def bodies(root):
    result = {}
    for path in sorted(root.rglob('*.java')):
        for index, identity in pattern.findall(path.read_text()):
            key = identity  # MethodKey includes owner, name, parameters, return.
            assert key not in result, key
            result[key] = {'identity': key, 'method_pool_index': int(index), 'unit': str(path.relative_to(root))}
    return result
a, b = bodies(before), bodies(after)
gained = [b[k] for k in sorted(b.keys()-a.keys())]
lost = [a[k] for k in sorted(a.keys()-b.keys())]
(out/'coverage-delta.json').write_text(json.dumps({'before': len(a), 'after': len(b), 'gained': gained, 'lost': lost}, indent=2)+'\n')

candidates = []
for path in sorted((after/'com/samsung/android/samsungpass').rglob('*.java')):
    text = path.read_text()
    if re.search(r'(?<![\w.])android\.', text) and 'complete_body=true' in text:
        candidates.append(path)
candidates.sort(key=lambda p: (p.stat().st_size, str(p)))
representatives = []
for i, path in enumerate(candidates[:40]):
    dest = out/f'unit-{i}'
    dest.mkdir(exist_ok=True)
    start = time.monotonic()
    command = [str(jdk), '-proc:none', '-classpath', android, '-sourcepath', str(dest), '-d', str(dest), str(path)]
    run = subprocess.run(command, capture_output=True, text=True, timeout=60)
    (dest/'javac.log').write_text(run.stderr)
    representatives.append({'unit': str(path.relative_to(after)), 'exit_code': run.returncode, 'seconds': time.monotonic()-start, 'command': command})
    if sum(r['exit_code'] == 0 for r in representatives) >= 5:
        break
args = out/'all-sources.args'
args.write_text(''.join('"'+str(p)+'"\n' for p in sorted(after.rglob('*.java'))))
dest = out/'all-classes'
dest.mkdir(exist_ok=True)
command = [str(jdk), '-J-Xmx8g', '-proc:none', '-Xmaxerrs', '5000', '-classpath', android, '-sourcepath', str(after), '-d', str(dest), '@'+str(args)]
start = time.monotonic()
if '--representatives-only' in sys.argv:
    previous = json.loads((out/'validation.json').read_text())['full_compile']
    full_exit = previous['exit_code']
    full_seconds = previous['seconds']
else:
    with (out/'full-javac.log').open('w') as log:
        run = subprocess.run(command, stdout=log, stderr=log, timeout=300)
    full_exit = run.returncode
    full_seconds = time.monotonic()-start
diagnostics = collections.Counter(re.findall(r'error: ([^\n]+)', (out/'full-javac.log').read_text()))
result = {'jdk': str(jdk), 'android_classpath': android, 'representative_original_units': representatives,
          'full_compile': {'command': command, 'exit_code': full_exit, 'seconds': full_seconds,
                           'diagnostic_cap': 5000, 'observed_diagnostic_categories_not_total_defects': diagnostics},
          'gained_identities': len(gained), 'lost_identities': len(lost), 'compiled_apk': False}
(out/'validation.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result, indent=2))
