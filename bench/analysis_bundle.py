#!/usr/bin/env python3
"""Codepuri Java + original DEX evidence bundle for bounded static analysis.

Symbolic invocation references are not resolved runtime dispatch or taint flow.
No APK code is executed. Java availability and original-code evidence stay separate.
"""
import argparse
import json
import re
import sqlite3
import subprocess
import zlib
from collections import defaultdict
from pathlib import Path

from method_comparison import FAILURE, KEY, STRING, SUCCESS, original_key, rust_string, sha256

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = '''
CREATE TABLE methods(id INTEGER PRIMARY KEY, entry TEXT, logical INTEGER, pool INTEGER,
 method_index INTEGER, identity TEXT, owner TEXT, name TEXT, signature TEXT, flags INTEGER,
 code_offset INTEGER, code_units INTEGER, features TEXT, decode_error TEXT,
 java_status TEXT, java_evidence TEXT, disassembly_zlib BLOB, raw_insns_zlib BLOB);
CREATE INDEX method_identity ON methods(identity);
CREATE INDEX method_owner ON methods(owner,name);
CREATE TABLE calls(caller_id INTEGER, offset INTEGER, opcode TEXT, target TEXT);
CREATE INDEX call_target ON calls(target);
CREATE INDEX call_caller ON calls(caller_id);
CREATE TABLE components(kind TEXT, name TEXT, explicit_exported TEXT, permission TEXT,
 enabled TEXT, actions TEXT, categories TEXT, data TEXT, manifest_line INTEGER, attributes TEXT, filters TEXT);
CREATE TABLE dex_entries(entry TEXT, logical INTEGER, root INTEGER, parse_error TEXT, metadata TEXT);
'''

def java_records(java: Path, log: Path | None):
    records = defaultdict(list)
    for path in java.rglob('*.java'):
        with path.open() as stream:
            for number, line in enumerate(stream, 1):
                success, failure = SUCCESS.search(line), FAILURE.search(line)
                if not (success or failure):
                    continue
                key = original_key(line)
                if not key:
                    continue
                item = {'path': str(path), 'line': number, 'method_index': int((success or failure)[1])}
                if success:
                    item.update(status='body' if success[3] == 'true' else 'declaration',
                                complete=success[2] == 'true', semantic=success[3] == 'true', dependency=success[4] == 'true')
                else:
                    match = KEY.search(line)
                    item.update(status='unavailable', reason=line[match.end():].strip().removeprefix('):').strip())
                records[key].append(item)
    if log and log.exists():
        text = log.read_text()
        block = re.search(r'unpublished_method_reasons:\s*\{(.*?)\n\s*\},', text, re.S)
        if block:
            for key, reason in re.findall(r'^\s*(' + STRING + r'):\s*(' + STRING + r'),\s*$', block[1], re.M):
                records[rust_string(key)].append({'status': 'unit_unpublished', 'reason': rust_string(reason), 'path': str(log)})
    return records

def java_status(items):
    bodies = [item for item in items if item['status'] == 'body']
    if bodies:
        return 'body_with_diagnostic' if any(not item['complete'] or not item['dependency'] for item in bodies) or any(item['status'] == 'unavailable' for item in items) else 'body'
    if any(item['status'] == 'declaration' for item in items):
        return 'declaration'
    if any(item['status'] == 'unavailable' for item in items):
        return 'unavailable'
    if any(item['status'] == 'unit_unpublished' for item in items):
        return 'unit_unpublished'
    return 'not_observed'

def components(text):
    result, stack = [], []
    current, current_indent = None, None
    current_filter, filter_indent, data_node = None, None, None
    for number, line in enumerate(text.splitlines(), 1):
        indent = len(line) - len(line.lstrip())
        element = re.match(r'\s*E:\s*(\S+)', line)
        if element:
            while stack and stack[-1][0] >= indent:
                stack.pop()
            if current and indent <= current_indent:
                current = None
            if current_filter and indent <= filter_indent:
                current_filter = None
            data_node = None
            tag = element[1]
            stack.append((indent, tag))
            if tag in {'application', 'activity', 'activity-alias', 'service', 'receiver', 'provider'}:
                current = {'kind': tag, 'name': None, 'exported': None, 'permission': None, 'enabled': None, 'actions': [], 'categories': [], 'data': [], 'line': number, 'attributes': {}, 'filters': []}
                current_indent = indent
                result.append(current)
            elif tag == 'intent-filter' and current:
                current_filter = {'line': number, 'actions': [], 'categories': [], 'data': []}
                filter_indent = indent
                current['filters'].append(current_filter)
            elif tag == 'data' and current_filter:
                data_node = {}
                current_filter['data'].append(data_node)
            continue
        attribute = re.match(r'\s*A:\s*android:(\w+)\([^)]*\)=(.*)', line)
        if not current or not attribute or not stack:
            continue
        key, value = attribute.groups()
        quoted = re.match(r'"([^"\n]*)"', value)
        value = quoted[1] if quoted else value
        tag = stack[-1][1]
        if tag == current['kind']:
            current['attributes'][key] = value
            if key in {'name', 'exported', 'permission', 'enabled'}:
                current[key] = value
        elif tag == 'action' and key == 'name':
            current['actions'].append(value)
            if current_filter:
                current_filter['actions'].append(value)
        elif tag == 'category' and key == 'name':
            current['categories'].append(value)
            if current_filter:
                current_filter['categories'].append(value)
        elif tag == 'data':
            current['data'].append({key: value})
            if data_node is not None:
                data_node[key] = value
    return result

def build(args):
    if args.out.exists():
        raise ValueError('analysis output must be a fresh directory')
    apk_hash = sha256(args.apk)
    provenance_path = args.java.parent / 'manifest.json'
    provenance = json.loads(provenance_path.read_text()) if provenance_path.exists() else None
    if provenance and provenance.get('sha256') != apk_hash:
        raise ValueError('Java export manifest does not match the supplied APK SHA-256')
    args.out.mkdir(parents=True)
    db = sqlite3.connect(args.out / 'analysis.sqlite')
    db.executescript(SCHEMA)
    records = java_records(args.java, args.java_log)
    if args.manifest:
        for item in components(args.manifest.read_text()):
            db.execute('INSERT INTO components VALUES(?,?,?,?,?,?,?,?,?,?,?)', (item['kind'], item['name'], item['exported'], item['permission'], item['enabled'], json.dumps(item['actions']), json.dumps(item['categories']), json.dumps(item['data']), item['line'], json.dumps(item['attributes']), json.dumps(item['filters'])))
    inventory = ROOT / 'target/release/examples/method_inventory'
    count = 0
    with (args.out / 'inventory.log').open('w') as stderr:
        process = subprocess.Popen([str(inventory), str(args.apk), '--disassembly'], stdout=subprocess.PIPE, stderr=stderr, text=True)
        try:
            for line in process.stdout:
                item = json.loads(line)
                if item['kind'] == 'dex':
                    db.execute('INSERT INTO dex_entries VALUES(?,?,?,?,?)', (item['entry'], item.get('logical'), item['root'], item.get('parse_error'), json.dumps(item)))
                    continue
                evidence = [record for record in records.get(item['identity'], []) if record.get('method_index', item['method_index']) == item['method_index']]
                cursor = db.execute('INSERT INTO methods VALUES(NULL,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                    (item['entry'], item['logical'], item['pool'], item['method_index'], item['identity'], item['owner'], item['name'], item['signature'], item['flags'], item['code_offset'], item['code_units'], json.dumps(item['features']), item['decode_error'], java_status(evidence), json.dumps(evidence), zlib.compress(item['disassembly'].encode()) if item['disassembly'] is not None else None, zlib.compress(bytes.fromhex(item['raw_insns_hex'])) if item['raw_insns_hex'] is not None else None))
                db.executemany('INSERT INTO calls VALUES(?,?,?,?)', [(cursor.lastrowid, call['offset'], call['opcode'], call['target']) for call in item['symbolic_calls']])
                count += 1
                if count % 10000 == 0:
                    db.commit()
                    print('Indexed methods:', count, flush=True)
            process.stdout.close()
            if process.wait():
                raise RuntimeError('inventory failed; inspect inventory.log')
        except BaseException:
            process.kill()
            process.wait()
            raise
    db.commit()
    db.execute('''UPDATE methods SET java_status='ambiguous_java_attribution'
        WHERE java_evidence!='[]' AND EXISTS (
          SELECT 1 FROM methods duplicate WHERE duplicate.identity=methods.identity
          AND duplicate.method_index=methods.method_index AND duplicate.id!=methods.id)''')
    db.commit()
    summary = {
        'apk': str(args.apk), 'apk_sha256': apk_hash, 'java_directory': str(args.java),
        'java_provenance_manifest': str(provenance_path) if provenance else None,
        'java_apk_binding': 'matching_export_manifest_sha256' if provenance else 'not_verified',
        'inventory_binary_sha256': sha256(inventory), 'methods': count,
        'dex_bodies': db.execute('SELECT COUNT(*) FROM methods WHERE code_offset>0').fetchone()[0],
        'disassembled_bodies': db.execute('SELECT COUNT(*) FROM methods WHERE disassembly_zlib IS NOT NULL').fetchone()[0],
        'decode_errors': db.execute('SELECT COUNT(*) FROM methods WHERE decode_error IS NOT NULL').fetchone()[0],
        'java_statuses': dict(db.execute('SELECT java_status,COUNT(*) FROM methods GROUP BY java_status')),
        'symbolic_invocations': db.execute('SELECT COUNT(*) FROM calls').fetchone()[0],
        'explicit_components': db.execute('SELECT COUNT(*) FROM components').fetchone()[0],
        'critical_lifecycle_methods': [],
        'limitations': ['Invocation targets are symbolic references, not resolved virtual dispatch or taint flow.', 'Native/abstract methods have no DEX body; native implementation is not recovered.', 'Raw offsets are 16-bit code units within the original code_item.', 'Java status records diagnostic evidence, not compilation/runtime correctness.'],
    }
    for row in db.execute("SELECT identity,java_status,code_units,disassembly_zlib IS NOT NULL,id FROM methods WHERE owner LIKE 'Lcom/elm/android/individual/%' AND (owner LIKE '%RoutingActivity;' OR owner LIKE '%MainActivity;' OR owner LIKE '%LoginActivity;') AND name IN ('onCreate','onNewIntent') ORDER BY identity"):
        summary['critical_lifecycle_methods'].append({'identity': row[0], 'java_status': row[1], 'code_units': row[2], 'dex_evidence': bool(row[3]), 'method_id': row[4]})
    summary['integrity'] = db.execute('PRAGMA quick_check').fetchone()[0]
    (args.out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    db.close()
    print(json.dumps(summary, indent=2), flush=True)

def show(args):
    db = sqlite3.connect(args.database.resolve().as_uri() + '?mode=ro', uri=True)
    rows = db.execute('SELECT id,identity,entry,method_index,code_offset,java_status,java_evidence,disassembly_zlib,raw_insns_zlib,decode_error FROM methods WHERE identity=?', (args.identity,)).fetchall()
    if len(rows) != 1:
        raise ValueError(f'identity has {len(rows)} matches; use SQLite entry/pool keys to disambiguate')
    row = rows[0]
    print(json.dumps({'method_id': row[0], 'identity': row[1], 'entry': row[2], 'method_index': row[3], 'code_offset': row[4], 'java_status': row[5], 'java_evidence': json.loads(row[6]), 'decode_error': row[9]}, indent=2))
    if row[7] is not None:
        print(zlib.decompress(row[7]).decode())
    elif row[8] is not None:
        print('Raw original instruction bytes:', zlib.decompress(row[8]).hex())
    else:
        print('No DEX body in the original definition; inspect native/abstract declaration flags.')

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest='command', required=True)
    export = subs.add_parser('build')
    export.add_argument('--apk', type=Path, required=True)
    export.add_argument('--java', type=Path, required=True)
    export.add_argument('--java-log', type=Path)
    export.add_argument('--manifest', type=Path)
    export.add_argument('--out', type=Path, required=True)
    query = subs.add_parser('show')
    query.add_argument('database', type=Path)
    query.add_argument('identity')
    args = parser.parse_args()
    build(args) if args.command == 'build' else show(args)

if __name__ == '__main__':
    main()
