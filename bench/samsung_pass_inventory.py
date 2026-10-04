#!/usr/bin/env python3
"""Static APK identity inventory and conservative export accounting; no app execution."""
import collections
import hashlib
import json
import pathlib
import re
import struct
import sys
import zipfile

apk = pathlib.Path(sys.argv[1])
root = pathlib.Path(sys.argv[2])
output = pathlib.Path(sys.argv[3])
counts = collections.Counter()
dexes = []
identities = []
with zipfile.ZipFile(apk) as z:
    entries = z.infolist()
    for entry in entries:
        if not re.fullmatch(r'classes(?:\d+)?\.dex', entry.filename):
            continue
        data = z.read(entry)
        u32 = lambda off: struct.unpack_from('<I', data, off)[0]
        def leb(off):
            value = shift = 0
            while True:
                b = data[off]
                off += 1
                value |= (b & 127) << shift
                if b < 128:
                    return value, off
                shift += 7
        string_count, string_off = u32(56), u32(60)
        strings = []
        for i in range(string_count):
            _, off = leb(u32(string_off + 4*i))
            strings.append(data[off:data.index(0, off)].decode('utf-8', 'replace'))
        types = [strings[u32(u32(68) + 4*i)] for i in range(u32(64))]
        protos = []
        for i in range(u32(72)):
            off = u32(76) + 12*i
            params_off = u32(off+8)
            params = [] if not params_off else [types[struct.unpack_from('<H', data, params_off+4+2*j)[0]] for j in range(u32(params_off))]
            protos.append('(' + ''.join(params) + ')' + types[u32(off+4)])
        methods = []
        for i in range(u32(88)):
            owner, proto, name = struct.unpack_from('<HHI', data, u32(92)+8*i)
            methods.append(types[owner] + '->' + strings[name] + protos[proto])
        local = collections.Counter()
        for i in range(u32(96)):
            off = u32(u32(100)+32*i+24)
            if not off:
                continue
            sizes = []
            for _ in range(4):
                n, off = leb(off)
                sizes.append(n)
            for _ in range(sizes[0]+sizes[1]):
                _, off = leb(off)
                _, off = leb(off)
            for size in sizes[2:]:
                index = 0
                for _ in range(size):
                    delta, off = leb(off)
                    index += delta
                    flags, off = leb(off)
                    code, off = leb(off)
                    identity = methods[index]
                    category = 'constructor' if '-><init>(' in identity else 'static_initializer' if '-><clinit>(' in identity else 'ordinary'
                    local[category] += 1
                    local['total'] += 1
                    local['code_bearing' if code else 'declaration_only'] += 1
                    identities.append({'dex': entry.filename, 'method_index': index, 'identity': identity, 'category': category, 'access_flags': flags, 'has_code': bool(code)})
        counts.update(local)
        dexes.append({'name': entry.filename, 'bytes': entry.file_size, 'sha256': hashlib.sha256(data).hexdigest(), 'classes': u32(96), 'defined_methods': dict(local), 'method_pool_entries_not_definition_count': u32(88)})
    native = [{'name': e.filename, 'bytes': e.file_size} for e in entries if e.filename.startswith('lib/') and e.filename.endswith('.so')]
    resource = [{'name': e.filename, 'bytes': e.file_size} for e in entries if e.filename in ('AndroidManifest.xml', 'resources.arsc')]
    assets = [e for e in entries if e.filename.startswith('assets/') and not e.is_dir()]
report_path = root/'report.txt'
if report_path.exists():
    report_lines = report_path.read_text().splitlines()
    summary = report_lines[0]
    report = {k: int(v) for k, v in re.findall(r'(\w+)=(\d+)(?=\s|$)', summary)}
else:
    # Session and profiler exports record the production JavaReport directly. Translate
    # its scalar names explicitly; never infer counts from directory contents.
    debug = (root/'report-debug.txt').read_text()
    raw = {k: int(v) for k, v in re.findall(r'(\w+): (\d+)(?=,|\s*})', debug)}
    scalar_names = {'complete_method_bodies': 'complete_methods',
                    'classes_generated': 'classes_emitted',
                    'classes_written': 'classes_emitted'}
    report = {k: raw.get(v, 0) for k, v in scalar_names.items()}
    report.update(raw)
    report_lines = []
    # Read the actual JSON-shaped Debug map independently of the following
    # field order. Other per-category maps may appear before aggregate reasons.
    marker = ', category_reasons: '
    if marker in debug:
        start = debug.index(marker) + len(marker)
        category_reasons, _ = json.JSONDecoder().raw_decode(debug[start:])
        for reason, count in category_reasons.items():
            report_lines.append(f'category_reason\t{count}\t{reason}')
    report['origin'] = 'JavaSession production report-debug scalar translation'
    profiler_summary = root/'summary.txt'
    if profiler_summary.exists():
        profiler_reports = [line.removeprefix('report=')
                            for line in profiler_summary.read_text().splitlines()
                            if line.startswith('report=')]
        if profiler_reports:
            if len(profiler_reports) != 1 or profiler_reports[0] != debug.rstrip('\n'):
                raise ValueError('profiler summary differs from captured production report')
            report['origin'] = 'java_profile_summary'
for line in report_lines:
    if line.startswith('unavailable_initialization_units='):
        report['unavailable_initialization_units'] = int(line.split('=')[1])
complete = report['complete_method_bodies'] + report['complete_constructors'] + report['complete_static_initializers']
rejects = report['unsupported_methods'] + report['unavailable_constructors']
assert counts['total'] == complete + rejects + report['declaration_only_methods'], (counts, report)
rules = collections.defaultdict(collections.Counter)
for line in report_lines:
    if line.startswith('category_reason\t'):
        _, n, category, reason = line.split('\t', 3)
        normalized = re.sub(r'(?:ValueId|InsnId|BlockId|EdgeId)\(\d+v\d+\)', 'ID', re.sub(r'MethodKey \{.*?\}', 'METHOD', reason))
        rules[category][normalized] += int(n)
samples = collections.defaultdict(list)
samsung = []
published = {}
rejected = {}
enum_units = {}
enum_method_statuses = {}
enum_marked_owners = set()
source_status = collections.Counter()
key_pattern = re.compile(r'MethodKey \{ owner: "([^"]+)", name: "([^"]+)", parameters: (\[[^\]]*\]), return_type: "([^"]+)" \}')
def original_identity(line):
    match = key_pattern.search(line)
    if not match:
        return None
    owner, name, params, result = match.groups()
    return owner + '->' + name + '(' + ''.join(json.loads(params)) + ')' + result
for path in sorted((root/'sources').rglob('*.java')):
    text = path.read_text()
    # A source-release requirement is metadata, not a class-kind marker.
    # Keep the same original enum/ordinary classification across 17/25 profiles.
    if text.startswith('// Required Java source release: 25\n'):
        text = text.split('\n', 1)[1]
    source_status['written_units'] += 1
    source_status['honest_subset_fallback_units'] += text.startswith('// HONEST SUBSET:')
    source_status['proven_enum_equivalence_units'] += text.startswith('// Proven enum initialization and synthetic method equivalence.')
    source_status['unavailable_initialization_marker_units'] += '// Original field initialization proof: unavailable.' in text
    unit = str(path.relative_to(root/'sources'))
    source_owner = 'L' + unit.removesuffix('.java') + ';'
    if text.startswith('// Proven enum initialization and synthetic method equivalence.'):
        enum_units[source_owner] = unit
    for index, status, category in re.findall(
            r'^// decomp-enum-method (\d+) (complete_body|proven_enum_equivalence|declaration_only) (ordinary|constructor|static_initializer)$',
            text, re.MULTILINE):
        marker_key = (source_owner, int(index))
        if marker_key in enum_method_statuses:
            raise ValueError('duplicate original enum method marker: ' + str(marker_key))
        enum_marked_owners.add(source_owner)
        enum_method_statuses[marker_key] = {'status': status, 'source_unit': unit,
                                           'marker_category': category}
    if 'package com.samsung.android.samsungpass' in text:
        samsung.append(str(path.relative_to(root/'sources')))
    for line in text.splitlines():
        identity = original_identity(line)
        if identity and '// original method pool index ' in line:
            assert identity not in published, identity
            published[identity] = {'status': 'complete_body' if 'complete_body=true' in line else 'declaration_only', 'source_unit': str(path.relative_to(root/'sources'))}
        if '// unavailable original method index ' in line:
            reason = line.rsplit('): ', 1)[-1]
            if identity:
                rejected[identity] = {'status': 'rejected_in_written_unit', 'source_unit': str(path.relative_to(root/'sources')), 'reason': reason}
            reason = re.sub(r'(?:ValueId|InsnId|BlockId|EdgeId)\(\d+v\d+\)', 'ID', reason)
            if len(samples[reason]) < 3:
                samples[reason].append({'unit': str(path.relative_to(root/'sources')), 'identity_and_reason': line.strip()})
# Separate diagnostic receipt: never upgrades a status or attests source delivery.
unpublished_reasons = {}
diagnostic_receipt = root / 'unpublished-methods.tsv'
if diagnostic_receipt.exists():
    lines = diagnostic_receipt.read_text().splitlines()
    if not lines or lines[0] != 'identity_hex\treason_hex':
        raise ValueError('invalid unpublished-method diagnostic receipt header')
    for line in lines[1:]:
        identity_hex, reason_hex = line.split('\t')
        identity = bytes.fromhex(identity_hex).decode('utf-8')
        reason = bytes.fromhex(reason_hex).decode('utf-8')
        if identity in unpublished_reasons or not reason:
            raise ValueError('duplicate/empty unpublished-method diagnostic: ' + identity)
        unpublished_reasons[identity] = reason

for row in identities:
    owner = row['identity'].split('->')[0]
    if owner in enum_marked_owners:
        evidence = enum_method_statuses.get((owner, row['method_index']))
        if evidence is None or evidence['marker_category'] != row['category']:
            raise ValueError('missing/mismatched exact original enum evidence: ' + row['identity'])
        if evidence['status'] == 'declaration_only' and (row['has_code'] or not row['access_flags'] & (0x100 | 0x400)):
            raise ValueError('non-genuine bodyless enum declaration: ' + row['identity'])
        row.update({k: v for k, v in evidence.items() if k != 'marker_category'})
        continue
    enum_status = {'status': 'proven_enum_equivalence', 'source_unit': enum_units[owner]} if owner in enum_units else {'status': 'rejected_without_written_unit'}
    row.update(published.get(row['identity'], rejected.get(row['identity'], enum_status)))
    if row['status'] == 'rejected_without_written_unit' and row['identity'] in unpublished_reasons:
        row['reason'] = unpublished_reasons[row['identity']]
        row['reason_origin'] = 'unpublished-method diagnostic receipt'

identity_status = collections.Counter(row['status'] for row in identities)
defined = {row['identity'] for row in identities}
unmatched = {k: v for k, v in published.items() if k not in defined}
if unmatched:
    output.with_suffix('.unmatched.json').write_text(json.dumps(unmatched, indent=2)+'\n')
assert identity_status['complete_body'] + identity_status['proven_enum_equivalence'] == complete, (identity_status, list(unmatched)[:5])
assert identity_status['declaration_only'] == report['declaration_only_methods'], identity_status
result = {'apk': str(apk), 'bytes': apk.stat().st_size, 'sha256': hashlib.sha256(apk.read_bytes()).hexdigest(),
          'package': 'com.samsung.android.samsungpass', 'version_name': '5.2.10.10', 'version_code': 521010900,
          'dex': dexes, 'original_defined_methods': dict(counts), 'export': report,
          'complete_bodies_all_original_methods_percent': 100*complete/counts['total'],
          'complete_bodies_code_bearing_percent': 100*complete/counts['code_bearing'],
          'rejected_all_original_methods_percent': 100*rejects/counts['total'],
          'rejected_count': rejects, 'complete_body_count': complete,
          'native_libraries_unsupported_not_decompiled': native, 'resources': resource,
          'assets': {'count': len(assets), 'uncompressed_bytes': sum(e.file_size for e in assets), 'decompiled': False},
          'category_reasons': {k: v.most_common() for k, v in rules.items()},
          'method_rejection_samples': dict(samples), 'original_samsung_package_units': samsung,
          'identity_status_counts': dict(identity_status),
          'source_status_counts': dict(source_status),
          'explicit_body_provenance_all_original_methods_percent': 100*identity_status['complete_body']/counts['total'],
          'coverage_note': 'Facade complete counts include proven Java enum/synthetic equivalence; explicit complete-body provenance is reported separately. Declarations are never complete bodies.',
          'source_root': str(root/'sources')}
result['method_coverage_jsonl'] = str(root/'method-coverage.jsonl')
output.write_text(json.dumps(result, indent=2)+'\n')
(root/'method-coverage.jsonl').write_text(''.join(json.dumps(row)+'\n' for row in identities))
print(json.dumps({k: result[k] for k in ('original_defined_methods', 'complete_body_count', 'rejected_count', 'complete_bodies_all_original_methods_percent', 'complete_bodies_code_bearing_percent')}))
