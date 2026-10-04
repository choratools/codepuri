#!/usr/bin/env python3
"""Group complete javac logs without treating unpublished input as external APIs."""
import collections
import json
import pathlib
import re
import sys
import zipfile

root, log, output = map(pathlib.Path, sys.argv[1:4])
rows = [json.loads(line) for line in (root/'method-coverage.jsonl').read_text().splitlines()]
owners = {r['identity'].split('->')[0][1:-1].replace('/', '.') for r in rows}
external = set()
for path in ['/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod',
             '/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar']:
    with zipfile.ZipFile(path) as archive:
        external.update(n.removeprefix('classes/')[:-6].replace('/', '.')
                        for n in archive.namelist() if n.endswith('.class'))
groups = collections.Counter()
messages = collections.Counter()
samples = collections.defaultdict(list)
blocks = re.split(r'(?=^/.*\.java:\d+: error:)', log.read_text(), flags=re.M)
for block in blocks:
    header = re.search(r'^/.*\.java:\d+: error: (.*)', block)
    if not header:
        continue
    message = header[1]
    messages[message] += 1
    symbol = re.search(r'  symbol:\s+class (\S+)', block)
    package = re.search(r'  location: package (\S+)', block)
    if message == 'cannot find symbol' and symbol and package:
        name = package[1]+'.'+symbol[1]
        if name in external and '$' in name:
            category = 'confirmed_external_binary_nested_source_name'
        elif name in owners:
            category = 'missing_input_class_or_alias'
        elif name in external:
            category = 'classpath_present_symbol_unresolved'
        else:
            category = 'missing_external_or_unmapped_class'
    elif message == 'cannot find symbol':
        category = 'missing_output_member_or_unresolved_symbol'
    elif message.startswith('package ') and message.endswith(' does not exist'):
        package_name = message[8:-15]
        category = ('missing_input_package' if any(n.startswith(package_name+'.') for n in owners)
                    else 'missing_external_package')
    elif 'constructor' in message:
        category = 'constructor_signature_or_implicit_parent'
    elif 'does not override abstract method' in message:
        category = 'missing_output_override_or_signature_alias'
    elif 'access' in message or 'is not public' in message:
        category = 'access'
    elif 'another module' in message:
        category = 'module_conflict'
    else:
        category = 'other_source_attribution'
    groups[category] += 1
    if len(samples[category]) < 12:
        samples[category].append(block.splitlines()[:8])
result = dict(total=sum(groups.values()), groups=groups, messages=messages, samples=samples,
              limits='Observed attribution only; unresolved member group requires individual metadata probes.')
output.write_text(json.dumps(result, indent=2)+'\n')
print(json.dumps(dict(total=result['total'], groups=groups), indent=2))
