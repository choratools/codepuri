#!/usr/bin/env python3
"""Diagnostic-only compile excluding two already observed invalid source units."""
import collections
import json
import pathlib
import re
import subprocess
import sys
import time

root, out = map(pathlib.Path, sys.argv[1:3])
out.mkdir(exist_ok=True)
excluded = {'com/samsung/android/samsungpass/certificate/viewmodel/CertificateViewModel.java', 'm0/b0.java'}
paths = [p for p in sorted(root.rglob('*.java')) if str(p.relative_to(root)) not in excluded]
args = out/'semantic-sources.args'
args.write_text(''.join('"'+str(p)+'"\n' for p in paths))
dest = out/'semantic-classes'
dest.mkdir(exist_ok=True)
empty = out/'empty-sourcepath'
empty.mkdir(exist_ok=True)
command = ['/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac', '-J-Xmx8g', '-proc:none', '-Xmaxerrs', '5000', '-classpath', '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar', '-sourcepath', str(empty), '-d', str(dest), '@'+str(args)]
start = time.monotonic()
with (out/'semantic-javac.log').open('w') as log:
    run = subprocess.run(command, stdout=log, stderr=log, timeout=300)
text = (out/'semantic-javac.log').read_text()
diagnostics = collections.Counter(re.findall(r'error: ([^\n]+)', text))
result = {'command': command, 'exit_code': run.returncode, 'seconds': time.monotonic()-start,
          'input_units': len(paths), 'excluded_known_invalid_units': sorted(excluded),
          'diagnostic_cap': 5000, 'cap_reached': sum(diagnostics.values()) == 5000,
          'observed_categories_not_total_defect_counts': diagnostics,
          'compiled_apk': False, 'scope': 'Partial diagnostic pass; original emitted files unchanged.'}
(out/'semantic-validation.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(result, indent=2))
