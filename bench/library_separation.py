#!/usr/bin/env python3
"""Build trusted-reference evidence and preview conservative library separation.

Fresh LibScout scores nominate artifacts; Codepuri exact class implementations
and globally distinctive body anchors determine whether source can be separated.
No unmatched code is asserted to be first-party. References are retained locally.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess
import time
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def score(text):
    value = float(text)
    if not 0 <= value <= 1:
        raise argparse.ArgumentTypeError('score must be finite and between 0 and 1')
    return value


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def run(argv, cwd, log):
    started = time.monotonic()
    with log.open('w') as stream:
        result = subprocess.run([str(arg) for arg in argv], cwd=cwd, stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f'command failed ({result.returncode}); see {log}')
    return {'argv': [str(arg) for arg in argv], 'log': str(log), 'wall_seconds': time.monotonic() - started}


def inventory(binary, path, output):
    with output.open('w') as stdout, output.with_suffix('.stderr').open('w') as stderr:
        result = subprocess.run([str(binary), str(path)], stdout=stdout, stderr=stderr)
    if result.returncode:
        raise RuntimeError(f'inventory failed: {output.with_suffix(".stderr")}')
    records = [json.loads(line) for line in output.read_text().splitlines()]
    assert records[0]['sha256'] == sha256(path)
    classes = {}
    for row in records[1:]:
        if row['owner'] in classes:
            raise ValueError(f'ambiguous class: {row["owner"]}')
        classes[row['owner']] = row
    return classes


def preview(app, reference, candidate_score, min_score):
    counts = Counter(anchor for row in app.values() for anchor in set(row['anchors']))
    decisions = []
    for owner, expected in reference.items():
        actual = app.get(owner)
        if not actual:
            continue
        matched = bool(actual['implementation_sha256']) and actual['implementation_sha256'] == expected['implementation_sha256']
        unique = sum(counts[anchor] == 1 for anchor in set(actual['anchors'])) if matched else 0
        eligible = candidate_score >= min_score and matched and unique > 0
        reason = ('profile score below threshold' if candidate_score < min_score else
                  'exact declarations/implementation not established' if not matched else
                  'no nontrivial globally distinctive implementation anchor' if not unique else
                  'exact original class implementation and distinctive anchor verified')
        decisions.append({'owner': owner, 'selected': eligible, 'reason': reason, 'unique_anchors': unique})
    return decisions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--candidates', type=Path, required=True, help='explicit official artifact URLs and version candidates')
    parser.add_argument('--out', type=Path, required=True, help='fresh result directory')
    parser.add_argument('--libscout', type=Path, required=True, help='built detector JAR')
    parser.add_argument('--libscout-cwd', type=Path, required=True, help='directory containing config/LibScout.toml')
    parser.add_argument('--java', type=Path, required=True)
    parser.add_argument('--d8', type=Path, required=True)
    parser.add_argument('--android-jar', type=Path, required=True)
    parser.add_argument('--inventory', type=Path, default=ROOT / 'target/release/examples/library_inventory')
    parser.add_argument('--min-score', type=score, default=0.95)
    args = parser.parse_args()
    for name in ['apk', 'libscout', 'libscout_cwd', 'java', 'd8', 'android_jar', 'inventory']:
        setattr(args, name, getattr(args, name).resolve())
    args.out = args.out.resolve()
    candidates = json.loads(args.candidates.read_text())
    if not candidates or not isinstance(candidates, list):
        parser.error('candidates must be a nonempty list')
    ids = [row['id'] for row in candidates]
    if len(ids) != len(set(ids)):
        parser.error('candidate identifiers must be unique')
    args.out.mkdir(parents=True, exist_ok=False)
    profiles = args.out / 'profiles'
    profiles.mkdir()
    artifacts, runs = [], []
    for index, candidate in enumerate(candidates):
        if not candidate['url'].startswith('https://'):
            raise ValueError('artifact URLs must use HTTPS')
        folder = args.out / f'artifact-{index}'
        folder.mkdir()
        suffix = '.aar' if candidate['url'].endswith('.aar') else '.jar'
        artifact = folder / ('artifact' + suffix)
        request = urllib.request.Request(candidate['url'], headers={'User-Agent': 'codepuri-library-evidence/1'})
        with urllib.request.urlopen(request, timeout=120) as response, artifact.open('wb') as output:
            while chunk := response.read(1024 * 1024):
                output.write(chunk)
        xml = ET.Element('library')
        for key, text in [('name', candidate['name']), ('category', candidate.get('category', 'Utilities')), ('version', candidate['version']),
                          ('comment', 'Reference artifact candidate; installed version and license are not inferred')]:
            ET.SubElement(xml, key).text = text
        xml_path = folder / 'library.xml'
        ET.ElementTree(xml).write(xml_path, encoding='utf-8', xml_declaration=True)
        runs.append(run([args.java, '-Xmx8g', '-jar', args.libscout, '-o', 'profile', '-a', args.android_jar,
                         '-x', xml_path, '-p', profiles, '-d', folder / 'logs', artifact], args.libscout_cwd, folder / 'profile.log'))
        profiles_after = list(profiles.rglob('*.libv'))
        if len(profiles_after) != index + 1:
            raise RuntimeError(f'profile generation did not produce one profile for {candidate["id"]}')
        jar = artifact
        if suffix == '.aar':
            jar = folder / 'classes.jar'
            with zipfile.ZipFile(artifact) as archive:
                jar.write_bytes(archive.read('classes.jar'))
                if any(name.startswith('libs/') and name.endswith('.jar') for name in archive.namelist()):
                    raise ValueError('AAR embedded secondary JARs require separate candidate references')
        dex_dir = folder / 'dex'
        dex_dir.mkdir()
        runs.append(run([args.d8, '--min-api', '24', '--lib', args.android_jar, '--output', dex_dir, jar], folder, folder / 'd8.log'))
        dex_archive = folder / 'reference-dex.zip'
        with zipfile.ZipFile(dex_archive, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            for dex in sorted(dex_dir.glob('classes*.dex')):
                archive.write(dex, dex.name)
        if not list(dex_dir.glob('classes*.dex')):
            raise RuntimeError('D8 produced no reference DEX')
        artifacts.append({**candidate, 'artifact_file': str(artifact), 'artifact_sha256': sha256(artifact),
                          'dex_file': str(dex_archive), 'dex_sha256': sha256(dex_archive)})
    runs.append(run([args.java, '-Xmx32g', '-jar', args.libscout, '-o', 'match', '-p', profiles,
                     '-a', args.android_jar, '-j', args.out / 'detector-results', '-d', args.out / 'detector-logs', args.apk],
                    args.libscout_cwd, args.out / 'match.log'))
    results = list((args.out / 'detector-results').rglob('*.json'))
    if len(results) != 1:
        raise RuntimeError('LibScout did not publish exactly one APK result')
    raw = json.loads(results[0].read_text())
    scores = {}
    for row in raw['lib_matches']:
        scores[(row['libName'], row['libVersion'])] = row['simScore']
    app = inventory(args.inventory, args.apk, args.out / 'apk-inventory.jsonl')
    evidence = {'format': 'codepuri-library-evidence-v1', 'apk_sha256': sha256(args.apk), 'references': []}
    summary = {'min_score': args.min_score, 'artifacts': [], 'selected_owners': [], 'runs': runs,
               'note': 'Candidates and artifact matches do not prove exact installed versions or licenses; changed/ambiguous classes stay.'}
    selected = set()
    for index, artifact in enumerate(artifacts):
        candidate_score = float(scores.get((artifact['name'], artifact['version']), 0))
        evidence['references'].append({key: artifact[key] for key in ['id', 'artifact_file', 'artifact_sha256', 'dex_file', 'dex_sha256']} |
            {'score': candidate_score, 'score_basis': 'LibScout 2.3.2 fresh artifact signature profile'})
        reference = inventory(args.inventory, artifact['dex_file'], args.out / f'reference-inventory-{index}.jsonl')
        decisions = preview(app, reference, candidate_score, args.min_score)
        selected.update(row['owner'] for row in decisions if row['selected'])
        summary['artifacts'].append({**artifact, 'profile_score': candidate_score, 'same_owner_classes': len(decisions),
                                    'exact_implementation_classes': sum(row['unique_anchors'] > 0 for row in decisions),
                                    'selected_classes': sum(row['selected'] for row in decisions),
                                    'reasons': dict(Counter(row['reason'] for row in decisions)), 'decisions': decisions})
    summary['selected_owners'] = sorted(selected)
    (args.out / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    (args.out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps({'evidence': str(args.out / 'evidence.json'), 'min_score': args.min_score,
                      'selected_classes': len(selected), 'artifacts': [{key: row[key] for key in ['id', 'profile_score', 'same_owner_classes', 'exact_implementation_classes', 'selected_classes']} for row in summary['artifacts']]}, indent=2))


if __name__ == '__main__':
    main()
