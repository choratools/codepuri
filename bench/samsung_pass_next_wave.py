#!/usr/bin/env python3
"""Run explicit local export/checks and retain uncapped logs and resource use."""
import json
import os
import pathlib
import re
import subprocess
import time

base = pathlib.Path(__file__).resolve().parent
out = pathlib.Path('/tmp/opencode/samsung-pass-next-wave')
out.mkdir(exist_ok=True)
jdk = '/home/jtjisgod/tools/jdk-17.0.13+11'
env = dict(os.environ, JAVA_HOME=jdk, PATH=jdk+'/bin:'+os.environ['PATH'])
checks = {}
def run(name, command):
    start = time.monotonic()
    with (out/(name+'.log')).open('w') as log:
        result = subprocess.run(['/usr/bin/time', '-v', '-o', str(out/(name+'.time'))]+command,
                                env=env, stdout=log, stderr=log, timeout=900)
    timing = (out/(name+'.time')).read_text()
    item = dict(command=command, exit_code=result.returncode, seconds=time.monotonic()-start,
                max_rss_kib=int(re.search(r'Maximum resident set size \(kbytes\): (\d+)', timing)[1]),
                log=str(out/(name+'.log')))
    checks[name] = item
    (out/'checks.json').write_text(json.dumps(checks, indent=2)+'\n')
    print(name, json.dumps(item), flush=True)
    return result.returncode

assert run('workspace-tests', ['cargo', 'test', '--workspace']) == 0
assert run('clippy', ['cargo', 'clippy', '-p', 'backend-java', '-p', 'decomp', '--all-targets', '--', '-D', 'warnings']) == 0
assert run('build', ['cargo', 'build', '--release', '-p', 'decomp', '--example', 'java_corpus']) == 0
assert run('export', ['target/release/examples/java_corpus', '--workers', '30',
    '--classpath', jdk+'/jmods/java.base.jmod', '--classpath', '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar',
    '--app', 'tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk']) == 0
print((out/'export.log').read_text(), flush=True)
