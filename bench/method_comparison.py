#!/usr/bin/env python3
"""Build an evidence-based SQLite method comparison from installed APK manifests.

This is diagnostic JSON/source export, not a paired throughput benchmark.
Missing method records are unobserved, never automatically decompilation failures.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sqlite3
import subprocess
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STRING = r'"(?:\\.|[^"\\])*"'
KEY = re.compile(r'MethodKey \{ owner: (' + STRING + r'), name: (' + STRING + r'), parameters: \[(.*?)\], return_type: (' + STRING + r') \}')
SUCCESS = re.compile(r'original method pool index (\d+);.*complete_body=(true|false); semantic_complete=(true|false); dependency_complete=(true|false)')
FAILURE = re.compile(r'unavailable original method index (\d+):')

SCHEMA = """
CREATE TABLE apps(id INTEGER PRIMARY KEY, package TEXT UNIQUE, label TEXT, apk TEXT, sha256 TEXT);
CREATE TABLE dex_entries(app_id INTEGER, entry TEXT, logical INTEGER, root INTEGER, pool INTEGER, bytes INTEGER, classes INTEGER, defined_methods INTEGER, parse_error TEXT, sha256 TEXT);
CREATE TABLE methods(id INTEGER PRIMARY KEY, app_id INTEGER, entry TEXT, logical INTEGER, pool INTEGER, method_index INTEGER, identity TEXT, owner TEXT, name TEXT, signature TEXT, class_flags INTEGER, flags INTEGER, code_offset INTEGER, code_units INTEGER, registers INTEGER, try_count INTEGER, instruction_count INTEGER, features TEXT, decode_error TEXT, raw_insns_sha256 TEXT, symbolic_code_sha256 TEXT);
CREATE INDEX method_identity ON methods(app_id,identity,method_index);
CREATE INDEX method_offset ON methods(app_id,entry,code_offset);
CREATE INDEX method_fingerprint ON methods(symbolic_code_sha256);
CREATE TABLE observations(id INTEGER PRIMARY KEY, method_id INTEGER, app_id INTEGER, tool TEXT, status TEXT, match_basis TEXT, artifact TEXT, locator TEXT, reason TEXT, source_sha256 TEXT, semantic_complete INTEGER, dependency_complete INTEGER, evidence TEXT);
CREATE INDEX observation_method ON observations(method_id,tool);
CREATE INDEX observation_app ON observations(app_id,tool,method_id);
CREATE TABLE unmatched(app_id INTEGER, tool TEXT, artifact TEXT, locator TEXT, reason TEXT, evidence TEXT);
CREATE TABLE tool_runs(app_id INTEGER, tool TEXT, argv TEXT, exit_code INTEGER, wall_seconds REAL, log TEXT, version TEXT, binary_sha256 TEXT, revision TEXT);
CREATE TABLE class_profiles(app_id INTEGER, pool INTEGER, descriptor TEXT, elapsed_seconds REAL, method_count INTEGER, stages TEXT, ssa TEXT);
CREATE TABLE validations(method_id INTEGER, tool TEXT, kind TEXT, status TEXT, artifact TEXT, command TEXT);
CREATE TABLE observation_summary(method_id INTEGER,tool TEXT,status TEXT,semantic_complete INTEGER,dependency_complete INTEGER,reason TEXT,artifact TEXT,PRIMARY KEY(method_id,tool));
CREATE VIEW method_comparison AS
SELECT m.*, c.status AS codepuri_status,j.status AS jadx_status,
 c.semantic_complete,c.dependency_complete,c.reason AS codepuri_reason,j.reason AS jadx_reason,
 c.artifact AS codepuri_artifact,j.artifact AS jadx_artifact
FROM methods m
LEFT JOIN observation_summary c ON c.method_id=m.id AND c.tool='codepuri'
LEFT JOIN observation_summary j ON j.method_id=m.id AND j.tool='jadx';
"""

def refresh_outcomes(db, app_id):
    db.execute('''INSERT OR REPLACE INTO observation_summary
    SELECT method_id,tool,
      CASE WHEN COUNT(DISTINCT status)=1 THEN MIN(status)
      WHEN tool='codepuri' AND SUM(status='body_emitted')>0 AND SUM(status='explicitly_unavailable')>0
      THEN 'body_emitted_with_diagnostic' ELSE 'multiple_statuses' END,
      MIN(semantic_complete),MIN(dependency_complete),GROUP_CONCAT(DISTINCT reason),MIN(artifact)
    FROM observations WHERE app_id=? GROUP BY method_id,tool''', (app_id,))
    db.commit()

def migrate_outcomes(db):
    kind = db.execute("SELECT type FROM sqlite_master WHERE name='observation_summary'").fetchone()
    if kind and kind[0] == 'view':
        db.execute('DROP VIEW method_comparison')
        db.execute('DROP VIEW observation_summary')
        start = SCHEMA.index('CREATE TABLE observation_summary')
        db.executescript(SCHEMA[start:])
    db.execute('CREATE INDEX IF NOT EXISTS observation_app ON observations(app_id,tool,method_id)')
    db.commit()

def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1048576), b''):
            h.update(chunk)
    return h.hexdigest()

def rust_string(token: str) -> str:
    token = token.replace('\\ u', '\\u')
    token = re.sub(r'\\u\{([0-9a-fA-F]+)\}', lambda m: json.dumps(chr(int(m[1], 16)))[1:-1], token)
    return json.loads(token.replace('\\0', '\\u0000'))

def original_key(line: str) -> str | None:
    match = KEY.search(line)
    if not match:
        return None
    owner, name, parameters, result = match.groups()
    parameters = ''.join(rust_string(t) for t in re.findall(STRING, parameters))
    return f'{rust_string(owner)}->{rust_string(name)}({parameters}){rust_string(result)}'

def observe(db, method_id, app_id, tool, status, basis, artifact, locator, reason=None, source=None, semantic=None, dependency=None, evidence=None):
    db.execute('INSERT INTO observations VALUES(NULL,?,?,?,?,?,?,?,?,?,?,?,?)',
               (method_id, app_id, tool, status, basis, str(artifact), str(locator), reason,
                hashlib.sha256(source.encode()).hexdigest() if source is not None else None,
                semantic, dependency, json.dumps(evidence, ensure_ascii=True)))

def unmatched(db, app_id, tool, artifact, locator, reason, evidence):
    db.execute('INSERT INTO unmatched VALUES(?,?,?,?,?,?)',
               (app_id, tool, str(artifact), str(locator), reason, json.dumps(evidence, ensure_ascii=True)))

def ingest_inventory(db, app_id: int, apk: Path, binary: Path, log: Path):
    with zipfile.ZipFile(apk) as archive:
        dex_hashes = {info.filename: hashlib.sha256(archive.read(info)).hexdigest()
                      for info in archive.infolist() if info.filename.endswith('.dex')}
    with log.open('w') as stderr:
        process = subprocess.Popen([str(binary), str(apk)], stdout=subprocess.PIPE, stderr=stderr, text=True)
        assert process.stdout is not None
        try:
            for line in process.stdout:
                record = json.loads(line)
                if record['kind'] == 'dex':
                    db.execute('INSERT INTO dex_entries VALUES(?,?,?,?,?,?,?,?,?,?)',
                               (app_id, record['entry'], record.get('logical'), record['root'], record.get('pool'), record['bytes'], record.get('classes'), record.get('defined_methods'), record.get('parse_error'), dex_hashes[record['entry']]))
                else:
                    raw = record.pop('raw_insns_hex')
                    canonical = record.pop('canonical_code')
                    db.execute('INSERT INTO methods VALUES(NULL,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                               (app_id, record['entry'], record['logical'], record['pool'], record['method_index'], record['identity'], record['owner'], record['name'], record['signature'], record['class_flags'], record['flags'], record['code_offset'], record['code_units'], record['registers'], record['try_count'], record['instruction_count'], json.dumps(record['features']), record['decode_error'], hashlib.sha256(bytes.fromhex(raw)).hexdigest() if raw is not None else None, hashlib.sha256(canonical.encode()).hexdigest() if canonical is not None else None))
            process.stdout.close()
            if process.wait() != 0:
                raise RuntimeError(f'inventory failed: {log}')
        except BaseException:
            process.kill()
            process.wait()
            raise
    db.commit()

def ingest_codepuri(db, app_id: int, output: Path):
    # Keep the method pool index: descriptors alone cannot disambiguate duplicate
    # owners from multiple DEX inputs. Unresolved duplicates are not guessed.
    identities = {}
    for row in db.execute('SELECT id,identity,method_index FROM methods WHERE app_id=? AND pool IS NOT NULL', (app_id,)):
        identities.setdefault((row[1], row[2]), []).append(row[0])
    for path in sorted(output.rglob('*.java')):
        with path.open(errors='strict') as stream:
            for number, line in enumerate(stream, 1):
                emitted, rejected = SUCCESS.search(line), FAILURE.search(line)
                if not (emitted or rejected):
                    continue
                identity = original_key(line)
                index = int((emitted or rejected)[1])
                candidates = identities.get((identity, index), [])
                if len(candidates) != 1:
                    unmatched(db, app_id, 'codepuri', path, number, 'missing_or_ambiguous_original_identity', line.strip())
                    continue
                if emitted:
                    semantic, dependency = emitted[3] == 'true', emitted[4] == 'true'
                    status = 'body_emitted' if semantic else 'declaration_emitted'
                    observe(db, candidates[0], app_id, 'codepuri', status, 'descriptor_and_pool_index', path, number,
                            semantic=semantic, dependency=dependency,
                            evidence={'complete_body': emitted[2] == 'true', 'identity_comment': line.strip()})
                else:
                    key_match = KEY.search(line)
                    reason = line[key_match.end():].strip().removeprefix('):').strip() if key_match else line.strip()
                    observe(db, candidates[0], app_id, 'codepuri', 'explicitly_unavailable', 'descriptor_and_pool_index', path, number, reason=reason, evidence=line.strip())
    # Whole-unit omissions have no source comments. Profiling emits an exact
    # hexadecimal identity receipt without claiming a body or declaration.
    receipt = output.parent / 'unpublished-methods.tsv'
    if receipt.exists():
        by_identity = {}
        for ident, ids in identities.items():
            by_identity.setdefault(ident[0], []).extend(ids)
        with receipt.open() as stream:
            next(stream)
            for number, line in enumerate(stream, 2):
                identity_hex, reason_hex = line.rstrip('\n').split('\t')
                identity = bytes.fromhex(identity_hex).decode()
                reason = bytes.fromhex(reason_hex).decode()
                ids = by_identity.get(identity, [])
                if len(ids) == 1 and not db.execute('SELECT 1 FROM observations WHERE method_id=? AND tool=?', (ids[0], 'codepuri')).fetchone():
                    observe(db, ids[0], app_id, 'codepuri', 'unit_unpublished', 'descriptor_receipt', receipt, number, reason=reason)
                elif len(ids) != 1:
                    unmatched(db, app_id, 'codepuri', receipt, number, 'ambiguous_unpublished_identity', identity)
    classes = output.parent / 'classes.tsv'
    if classes.exists():
        with classes.open() as stream:
            next(stream)
            for line in stream:
                pool, _, descriptor, _, elapsed, methods, stages, ssa = line.rstrip('\n').split('\t')
                db.execute('INSERT INTO class_profiles VALUES(?,?,?,?,?,?,?)', (app_id, int(pool), descriptor, float(elapsed), int(methods), stages, ssa))
    db.commit()

def walk_classes(record, inherited_dex=None):
    dex = record.get('dex', inherited_dex)
    yield record, dex
    for child in record.get('inner-classes', []):
        yield from walk_classes(child, dex)

def ingest_jadx(db, app_id: int, output: Path):
    offsets, identities = {}, {}
    for method_id, entry, code_offset, identity, signature in db.execute('SELECT id,entry,code_offset,identity,name||signature FROM methods WHERE app_id=?', (app_id,)):
        identities.setdefault(identity, []).append((method_id, entry, code_offset))
        if code_offset:
            offsets.setdefault((entry, code_offset), []).append((method_id, signature))
    sources = output / 'sources' if (output / 'sources').exists() else output
    for path in sorted(sources.rglob('*.json')):
        try:
            with path.open() as stream:
                document = json.load(stream)
        except (json.JSONDecodeError, UnicodeError) as error:
            unmatched(db, app_id, 'jadx', path, 'file', 'invalid_json_artifact', str(error))
            continue
        if not isinstance(document, dict) or 'methods' not in document:
            unmatched(db, app_id, 'jadx', path, 'file', 'not_class_json', None)
            continue
        for cls, dex in walk_classes(document):
            owner = 'L' + cls.get('name', '').replace('.', '/') + ';'
            for position, method in enumerate(cls.get('methods', [])):
                signature = method.get('signature', '')
                locator = f'{cls.get("name", "?")}/methods/{position}'
                code_offset = int(method.get('offset', '0'), 0)
                # JADX JSON offsets identify original code_item boundaries;
                # require the full raw method signature too, never alias alone.
                candidates = [row[0] for row in offsets.get((dex, code_offset), []) if row[1] == signature]
                basis = 'dex_code_offset_and_raw_signature'
                if not candidates and code_offset == 0:
                    candidates = [row[0] for row in identities.get(owner + '->' + signature, []) if row[1] == dex and row[2] == 0]
                    basis = 'dex_and_descriptor'
                if len(candidates) != 1:
                    unmatched(db, app_id, 'jadx', path, locator, 'missing_or_ambiguous_original_identity', {'dex': dex, 'offset': code_offset, 'signature': signature, 'class': cls.get('name')})
                    continue
                text = method.get('declaration', '') + '\n' + '\n'.join(line.get('code', '') for line in method.get('lines', []))
                markers = [marker for marker in ('Method not decompiled', 'Code decompiled incorrectly') if marker in text]
                flags = method.get('access-flags', 0)
                status = 'diagnostic_marked' if markers else ('declaration_emitted' if flags & 0x500 else 'body_observed_no_marker')
                observe(db, candidates[0], app_id, 'jadx', status, basis, path, locator,
                        reason='; '.join(markers) if markers else None, source=text,
                        evidence={'dex': dex, 'offset': code_offset, 'signature': signature, 'markers': markers})
    db.commit()

def run_tool(db, app_id, tool, argv, log, binary, version, revision):
    start = time.monotonic()
    with log.open('w') as stream:
        process = subprocess.run(argv, stdout=stream, stderr=subprocess.STDOUT)
    db.execute('INSERT INTO tool_runs VALUES(?,?,?,?,?,?,?,?,?)',
               (app_id, tool, json.dumps(argv), process.returncode, time.monotonic() - start, str(log), version, sha256(binary), revision))
    db.commit()
    return process.returncode

def report(db, out: Path):
    summaries = []
    for app_id, package, label in db.execute('SELECT id,package,label FROM apps ORDER BY id').fetchall():
        total, roots = db.execute('SELECT COUNT(*),SUM(pool IS NOT NULL) FROM methods WHERE app_id=?', (app_id,)).fetchone()
        tools = {tool: dict(db.execute('SELECT status,COUNT(*) FROM observations WHERE app_id=? AND tool=? GROUP BY status', (app_id, tool))) for tool in ('codepuri', 'jadx')}
        matrix = [dict(codepuri=c or 'unobserved', jadx=j or 'unobserved', methods=n) for c, j, n in db.execute('SELECT codepuri_status,jadx_status,COUNT(*) FROM method_comparison WHERE app_id=? AND pool IS NOT NULL GROUP BY codepuri_status,jadx_status', (app_id,))]
        summaries.append({'package': package, 'label': label, 'methods_all_entries': total, 'methods_codepuri_scope': roots, 'observations': tools, 'root_scope_matrix': matrix})
    duplicates = db.execute('SELECT COUNT(*) FROM (SELECT symbolic_code_sha256 FROM methods WHERE symbolic_code_sha256 IS NOT NULL AND code_units>=8 GROUP BY symbolic_code_sha256 HAVING COUNT(DISTINCT app_id)>1)').fetchone()[0]
    result = {'schema_version': 1, 'apps': summaries, 'cross_app_symbolic_code_groups_min_8_units': duplicates,
              'notes': ['JADX JSON export and Codepuri profiled export are diagnostic runs, not paired throughput samples.', 'No marker is not proof of correctness; unobserved does not mean failure.', 'Class timing is never divided across methods. Per-method timing/repetition counts and compiler/runtime validation remain unmeasured.', 'Symbolic fingerprints preserve register numbering and branch/payload layout; they are duplicate candidates, not semantic equivalence proofs.']}
    (out / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
    rows = ['# Method comparison dataset', '', '| App | All DEX methods | Codepuri scope | Codepuri records | JADX records |', '|---|---:|---:|---:|---:|']
    for app in summaries:
        rows.append(f"| {app['label']} | {app['methods_all_entries']:,} | {app['methods_codepuri_scope']:,} | {sum(app['observations']['codepuri'].values()):,} | {sum(app['observations']['jadx'].values()):,} |")
    rows += ['', f'Cross-app fingerprint candidate groups (minimum 8 code units): {duplicates:,}.', '', *result['notes']]
    (out / 'SUMMARY.md').write_text('\n'.join(rows) + '\n')

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, action='append', required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--package', action='append', help='Select package(s); default is all manifest entries')
    parser.add_argument('--classpath', type=Path, required=True)
    parser.add_argument('--workers', type=int, default=30)
    parser.add_argument('--resume', action='store_true', help='Resume committed artifacts with matching binary/classpath configuration')
    parser.add_argument('--jadx', type=Path, default=Path('/home/jtjisgod/tools/jadx/bin/jadx'))
    args = parser.parse_args()
    if args.workers < 1:
        parser.error('workers must be positive')
    if args.out.exists() and not args.resume:
        parser.error('output directory must not already exist')
    if args.resume and not (args.out / 'comparison.sqlite').exists():
        parser.error('resume requires an existing dataset')
    args.out.mkdir(parents=True, exist_ok=args.resume)
    manifests = [app for path in args.manifest for app in json.loads(path.read_text())]
    if args.package:
        manifests = [app for app in manifests if app['package'] in args.package]
        missing = set(args.package) - {app['package'] for app in manifests}
        if missing:
            parser.error(f'unknown packages: {sorted(missing)}')
    db = sqlite3.connect(args.out / 'comparison.sqlite')
    if args.resume:
        migrate_outcomes(db)
    else:
        db.executescript(SCHEMA)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = subprocess.check_output(['git', 'diff', '--', 'crates'], cwd=ROOT)
    revision += ':working-diff-sha256=' + hashlib.sha256(dirty).hexdigest()
    jadx_version = subprocess.check_output([str(args.jadx), '--version'], text=True).strip()
    inventory = ROOT / 'target/release/examples/method_inventory'
    codepuri = ROOT / 'target/release/examples/java_profile'
    configuration = {
        'schema_version': 1, 'revision': revision, 'workers': args.workers,
        'classpath': str(args.classpath), 'classpath_sha256': sha256(args.classpath),
        'inventory_binary_sha256': sha256(inventory), 'codepuri_binary_sha256': sha256(codepuri),
        'jadx_version': jadx_version, 'jadx_launcher_sha256': sha256(args.jadx),
        'jadx_output_format': 'json', 'codepuri_export': 'profiled_with_checksum',
        'source_manifests': [{'path': str(p), 'sha256': sha256(p)} for p in args.manifest],
        'missing_measurements': ['per-method wall/CPU time', 'analysis repetition counts', 'compile correctness', 'runtime equivalence'],
    }
    if args.resume:
        previous = json.loads((args.out / 'configuration.json').read_text())
        for key in ['workers', 'classpath_sha256', 'inventory_binary_sha256', 'codepuri_binary_sha256', 'jadx_version', 'jadx_launcher_sha256']:
            if previous[key] != configuration[key]:
                raise RuntimeError(f'resume configuration mismatch: {key}')
    else:
        (args.out / 'configuration.json').write_text(json.dumps(configuration, indent=2) + '\n')
    for app in manifests:
        apk = Path(app.get('local_path', app.get('apk', '')))
        digest = sha256(apk)
        if app.get('sha256') != digest:
            raise RuntimeError(f'APK hash mismatch: {apk}')
        existing = db.execute('SELECT id,sha256 FROM apps WHERE package=?', (app['package'],)).fetchone()
        if existing:
            if existing[1] != digest:
                raise RuntimeError('stored APK digest differs')
            app_id = existing[0]
            counts = dict(db.execute('SELECT tool,COUNT(*) FROM observations WHERE app_id=? GROUP BY tool', (app_id,)))
            if counts.get('codepuri') and counts.get('jadx'):
                refresh_outcomes(db, app_id)
                print('Resumed completed:', app['label'], flush=True)
                continue
        else:
            cursor = db.execute('INSERT INTO apps VALUES(NULL,?,?,?,?)', (app['package'], app['label'], str(apk), digest))
            app_id = cursor.lastrowid
        local = args.out / app['package']
        local.mkdir(exist_ok=args.resume)
        print('Inventory:', app['label'], flush=True)
        if not existing:
            ingest_inventory(db, app_id, apk, inventory, local / 'inventory.log')
        cp_out = local / 'codepuri'
        print('Codepuri:', app['label'], flush=True)
        cp_run = db.execute("SELECT exit_code FROM tool_runs WHERE app_id=? AND tool='codepuri'", (app_id,)).fetchone()
        cp_exit = cp_run[0] if cp_run else run_tool(db, app_id, 'codepuri', [str(codepuri), str(apk), str(cp_out), '--workers', str(args.workers), '--classpath', str(args.classpath)], local / 'codepuri.log', codepuri, 'java_profile', revision)
        if cp_exit:
            raise RuntimeError(f'Codepuri failed: {local / "codepuri.log"}')
        if not db.execute("SELECT 1 FROM observations WHERE app_id=? AND tool='codepuri' LIMIT 1", (app_id,)).fetchone():
            ingest_codepuri(db, app_id, cp_out / 'sources')
        j_out = local / 'jadx'
        print('JADX JSON:', app['label'], flush=True)
        j_run = db.execute("SELECT exit_code FROM tool_runs WHERE app_id=? AND tool='jadx'", (app_id,)).fetchone()
        j_exit = j_run[0] if j_run else run_tool(db, app_id, 'jadx', [str(args.jadx), '--no-res', '-j', str(args.workers), '--output-format', 'json', '-d', str(j_out), str(apk)], local / 'jadx.log', args.jadx, jadx_version, None)
        # JADX exit 1 is permitted only with its explicit completion receipt.
        if j_exit and not re.search(r'finished with errors, count:\s*\d+', (local / 'jadx.log').read_text()):
            raise RuntimeError(f'JADX did not finish: {local / "jadx.log"}')
        if not db.execute("SELECT 1 FROM observations WHERE app_id=? AND tool='jadx' LIMIT 1", (app_id,)).fetchone():
            ingest_jadx(db, app_id, j_out)
        refresh_outcomes(db, app_id)
        print('Recorded:', app['label'], flush=True)
    report(db, args.out)
    db.close()
    print('Dataset:', args.out / 'comparison.sqlite', flush=True)

if __name__ == '__main__':
    main()
