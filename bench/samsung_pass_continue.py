#!/usr/bin/env python3
"""Exact-source uncapped parser/compile gates and original-identity delta."""
import collections
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import time

parser_args = argparse.ArgumentParser()
parser_args.add_argument('before', type=pathlib.Path, nargs='?',
                         help='Optional previous export; omit when its identity ledger is unavailable')
parser_args.add_argument('after', type=pathlib.Path)
parser_args.add_argument('out', type=pathlib.Path)
parser_args.add_argument('--jdk-home', type=pathlib.Path,
                         default=pathlib.Path('/home/jtjisgod/tools/jdk-17.0.13+11'))
parser_args.add_argument('--source-release', choices=('17', '25'), default='17')
options = parser_args.parse_args()
before, after, out = options.before, options.after, options.out
out.mkdir(parents=True, exist_ok=True)
jdk = options.jdk_home/'bin'
android = '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar'
def identities(root):
    return {r['identity']: r for r in map(json.loads, (root/'method-coverage.jsonl').read_text().splitlines())}
a = identities(before) if before is not None else None
b = identities(after)
accepted = {'complete_body', 'proven_enum_equivalence'}
statuses = collections.Counter(r['status'] for r in b.values())
gained = lost = None
identity_delta = None
if a is not None:
    assert a.keys() == b.keys()
    transitions = collections.Counter((a[k]['status'], b[k]['status']) for k in a)
    changed = [{'identity': k, 'before': a[k], 'after': b[k]} for k in sorted(a) if a[k]['status'] != b[k]['status']]
    gained = [k for k in sorted(a) if a[k]['status'] not in accepted and b[k]['status'] in accepted]
    lost = [k for k in sorted(a) if a[k]['status'] in accepted and b[k]['status'] not in accepted]
    identity_delta = out/'identity-delta.json'
    identity_delta.write_text(json.dumps({'transitions': {f'{x} -> {y}': n for (x,y),n in transitions.items()}, 'changed': changed}, indent=2)+'\n')
sources = after/'sources'
paths = sorted(sources.rglob('*.java'))
hashes = {str(p.relative_to(sources)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
def run(name, command):
    start = time.monotonic()
    with (out/f'{name}.log').open('w') as log:
        result = subprocess.run(['/usr/bin/time', '-v', '-o', str(out/f'{name}.time')]+command, stdout=log, stderr=log, timeout=600)
    timing = (out/f'{name}.time').read_text()
    return {'command': list(map(str, command)), 'exit_code': result.returncode, 'seconds': time.monotonic()-start,
            'max_rss_kib': int(re.search(r'Maximum resident set size \(kbytes\): (\d+)', timing)[1]), 'log': str(out/f'{name}.log')}
helper = pathlib.Path(__file__).with_name('SamsungPassParseGate.java')
compile_helper = run('parse-helper', [str(jdk/'javac'), '-d', str(out), str(helper)])
assert compile_helper['exit_code'] == 0
parser = run('syntax-gate', [str(jdk/'java'), '-Xmx4g', '-cp', str(out), 'SamsungPassParseGate', str(sources), options.source_release])
args = out/'sources.args'
args.write_text(''.join('"'+str(p)+'"\n' for p in paths))
dest = out/'classes'
dest.mkdir(exist_ok=True)
empty = out/'empty-sourcepath'
empty.mkdir(exist_ok=True)
full = run('full-javac', [str(jdk/'javac'), '-J-Xmx8g', '--release', options.source_release, '-proc:none', '-Xmaxerrs', '2147483647', '-classpath', android, '-sourcepath', str(empty), '-d', str(dest), '@'+str(args)])
diagnostics = collections.Counter(re.findall(r'error: ([^\n]+)', pathlib.Path(full['log']).read_text()))
assert hashes == {str(p.relative_to(sources)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
representatives = []
for relative in ['com/samsung/android/samsungpass/passservice/bi/BiInfoColumns.java', 'com/samsung/android/samsungpass/ocr/nfc/e.java', 'com/samsung/android/samsungpass/ocr/nfc/d.java', 'com/samsung/android/samsungpass/passservice/common/storage/db/AppInfoColumns.java', 'com/samsung/android/samsungpass/passservice/common/storage/db/KeyInfoColumns.java', 'm0/q0$type.java', 'c/c$type.java', 'm0/b0$type.java', 'com/samsung/android/samsungpass/certificate/viewmodel/CertificateViewModel.java']:
    # Names may be aliased; consult unchanged source paths rather than rewrite.
    path = sources/relative
    if not path.exists():
        parent, leaf = relative.rsplit('/', 1)
        path = sources/parent/(leaf[:-5]+'$type.java')
    if not path.exists():
        representatives.append({'requested_unit': relative, 'not_published': True})
        continue
    name = 'representative-'+str(len(representatives))
    item = run(name, [str(jdk/'javac'), '-proc:none', '-classpath', android, '-sourcepath', str(empty), '-d', str(dest), str(path)])
    item['source_path'] = str(path)
    item['sha256'] = hashes[str(path.relative_to(sources))]
    representatives.append(item)
result = {'source_root': str(sources), 'input_units': len(paths), 'identity_status_counts': statuses, 'baseline_available': a is not None, 'identity_delta': str(identity_delta) if identity_delta else None, 'gained_bodies': len(gained) if gained is not None else None, 'lost_bodies': len(lost) if lost is not None else None, 'syntax_gate': parser, 'full_compile': full, 'diagnostics': diagnostics, 'diagnostic_count': sum(diagnostics.values()), 'diagnostic_cap': 2147483647, 'diagnostic_cap_reached': sum(diagnostics.values()) >= 2147483647, 'source_hashes_unchanged': True, 'representatives': representatives, 'compiled_apk': False}
(out/'validation.json').write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k != 'diagnostics'}, indent=2))
