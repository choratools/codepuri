#!/usr/bin/env python3
"""Export APK + original method inventory into an offline analysis filesystem."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import sqlite3
import subprocess
import struct
import zipfile
import zlib


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def safe_entry(name):
    path = PurePosixPath(name)
    if path.is_absolute() or '..' in path.parts or '\\' in name:
        raise ValueError(f'unsafe APK entry: {name!r}')
    return path


def command(args, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open('wb') as out:
        result = subprocess.run(args, stdout=out, stderr=subprocess.PIPE)
    return {'command': args, 'exit_code': result.returncode,
            'stderr': result.stderr.decode(errors='replace'), 'output': str(target)}


def native_dump(source, target):
    with source.open('rb') as stream:
        header = stream.read(20)
    if header[:4] != b'\x7fELF':
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text('Not a directly parseable ELF image. Original bytes are preserved.\n'
                          + 'Header: ' + header.hex() + '\n')
        return {'command': ['classify-native-container', str(source)], 'exit_code': 0,
                'output': str(target), 'format': 'opaque-non-ELF', 'header': header.hex(),
                'implementation_recovered': False}
    machine = struct.unpack('<H' if header[5] == 1 else '>H', header[18:20])[0]
    tool = 'aarch64-linux-gnu-objdump' if machine in (40, 183) else 'objdump'
    result = command([tool, '-d', '-C', str(source)], target)
    if result['exit_code'] and machine == 183:
        from elftools.elf.elffile import ELFFile
        import capstone
        decoder = capstone.Cs(capstone.CS_ARCH_ARM64, capstone.CS_MODE_LITTLE_ENDIAN)
        with source.open('rb') as stream, target.open('w') as output:
            elf = ELFFile(stream)
            output.write('# ELF executable-section evidence; undecoded words preserved.\n')
            for section in elf.iter_sections():
                if not section['sh_flags'] & 4:
                    continue
                output.write(f"\nSECTION {section.name} address=0x{section['sh_addr']:x}\n")
                data = section.data()
                for offset in range(0, len(data), 4):
                    address = section['sh_addr'] + offset
                    chunk = data[offset:offset+4]
                    instruction = next(decoder.disasm(chunk, address, count=1), None)
                    text = f'{instruction.mnemonic} {instruction.op_str}' if instruction else '.raw ' + chunk.hex()
                    output.write(f'{address:016x}: {chunk.hex()} {text}\n')
        return {'command': ['capstone-elf-executable-sections', str(source)], 'exit_code': 0,
                'output': str(target), 'bfd_failure': result, 'implementation_recovered': False}
    return result


def refresh_decoding(args):
    path = args.out / 'receipt.json'
    receipt = json.loads(path.read_text())
    retries = []
    for tool in receipt['tools']:
        if tool['exit_code'] == 0 or tool.get('superseded'):
            continue
        target = Path(tool['output'])
        if tool['command'][0] in ('llvm-objdump', 'aarch64-linux-gnu-objdump', 'objdump'):
            retries.append(native_dump(Path(tool['command'][-1]), target))
            tool['superseded'] = True
        elif tool['command'][0] == 'readelf':
            source = Path(tool['command'][-1])
            with source.open('rb') as stream:
                header = stream.read(4)
            if header != b'\x7fELF':
                retries.append(native_dump(source, target))
                tool['superseded'] = True
        elif 'xmltree' in tool['command']:
            relative = target.relative_to(args.out / 'resources')
            source = args.out / 'original' / str(relative).removesuffix('.tree.txt')
            with source.open('rb') as stream:
                header = stream.read(128)
            if header.lstrip().startswith(b'<'):
                shutil.copyfile(source, target)
                retries.append({'command': ['preserve-text-xml', str(source)], 'exit_code': 0,
                                'output': str(target), 'format': 'plain XML'})
                tool['superseded'] = True
        elif 'resources' in tool['command']:
            with zipfile.ZipFile(tool['command'][-1]) as archive:
                if 'resources.arsc' not in archive.namelist():
                    tool['expected_absence'] = 'archive has no resources.arsc'
    receipt['tools'].extend(retries)
    path.write_text(json.dumps(receipt, indent=2) + '\n')
    print('decoder retries:', len(retries))


def build(args):
    args.out.mkdir(parents=True, exist_ok=False)
    db = sqlite3.connect(args.database.resolve().as_uri() + '?mode=ro', uri=True)
    db.row_factory = sqlite3.Row
    summary_path = args.database.parent / 'summary.json'
    binding = json.loads(summary_path.read_text())
    apk_hash = digest(args.apk)
    if binding['apk_sha256'] != apk_hash:
        raise ValueError('inventory/APK SHA-256 mismatch')
    tools, entries, java_roots = [], [], []
    for number, root in enumerate(args.java):
        dest = args.out / 'java' / str(number)
        shutil.copytree(root, dest, copy_function=shutil.copy2)
        java_roots.append({'source': str(root), 'directory': str(dest.relative_to(args.out)),
                           'binding': 'source snapshot; original method identities retained in comments'})
    with zipfile.ZipFile(args.apk) as archive:
        seen = set()
        for entry in archive.infolist():
            relative = safe_entry(entry.filename)
            if entry.is_dir():
                continue
            if entry.filename in seen:
                raise ValueError(f'duplicate APK entry: {entry.filename}')
            seen.add(entry.filename)
            dest = args.out / 'original' / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(entry) as source, dest.open('xb') as output:
                shutil.copyfileobj(source, output)
            entries.append({'entry': entry.filename, 'bytes': entry.file_size, 'sha256': digest(dest)})
            if entry.filename.endswith('.xml'):
                tools.append(command([str(args.aapt2), 'dump', 'xmltree', str(args.apk), '--file', entry.filename],
                                     args.out / 'resources' / (entry.filename + '.tree.txt')))
            if entry.filename.endswith('.so'):
                tools.append(command(['readelf', '-aW', str(dest)], args.out / 'native' / (entry.filename + '.elf.txt')))
                tools.append(native_dump(dest, args.out / 'native' / (entry.filename + '.asm.txt')))
    tools.append(command([str(args.aapt2), 'dump', 'resources', str(args.apk)], args.out / 'resources/table.txt'))
    components = [dict(row) for row in db.execute('SELECT * FROM components')]
    (args.out / 'components.json').write_text(json.dumps(components, indent=2) + '\n')
    evidence_dir = args.out / 'methods'
    evidence_dir.mkdir()
    count = bodies = errors = 0
    next_progress = 10000
    with (args.out / 'methods.jsonl').open('w') as index:
        # Owner grouping bounds open file descriptors; full descriptor remains in every record.
        for owner_row in db.execute('SELECT DISTINCT owner FROM methods ORDER BY owner'):
            owner = owner_row[0]
            key = hashlib.sha256(owner.encode()).hexdigest()
            directory = evidence_dir / key[:2]
            directory.mkdir(exist_ok=True)
            metadata_path = directory / (key + '.jsonl')
            assembly_path = directory / (key + '.dex.txt')
            with metadata_path.open('x') as metadata, assembly_path.open('x') as assembly:
                for row in db.execute('SELECT * FROM methods WHERE owner=? ORDER BY entry,method_index,id', (owner,)):
                    item = {name: row[name] for name in ['id', 'entry', 'logical', 'pool', 'method_index', 'identity',
                            'owner', 'name', 'signature', 'flags', 'code_offset', 'code_units', 'decode_error']}
                    item['legacy_java_status'] = row['java_status']
                    item['java_status_note'] = 'inventory snapshot; consult latest Java roots for current publication flags'
                    item['symbolic_calls'] = [dict(call) for call in db.execute('SELECT offset,opcode,target FROM calls WHERE caller_id=? ORDER BY offset', (row['id'],))]
                    item['assembly_offset'] = assembly.tell()
                    assembly.write(f"\n=== {row['entry']} method_index={row['method_index']} id={row['id']} {row['identity']} ===\n")
                    if row['disassembly_zlib'] is not None:
                        assembly.write(zlib.decompress(row['disassembly_zlib']).decode() + '\n')
                        bodies += 1
                    elif row['raw_insns_zlib'] is not None:
                        assembly.write('RAW ORIGINAL CODE UNITS: ' + zlib.decompress(row['raw_insns_zlib']).hex() + '\n')
                    else:
                        assembly.write('NO ORIGINAL DEX BODY: inspect native/abstract flags and original libraries.\n')
                    item['evidence_file'] = str(metadata_path.relative_to(args.out))
                    item['assembly_file'] = str(assembly_path.relative_to(args.out))
                    metadata.write(json.dumps(item) + '\n')
                    index.write(json.dumps({name: item[name] for name in ['id', 'entry', 'method_index', 'identity', 'owner', 'evidence_file', 'assembly_file', 'assembly_offset']}) + '\n')
                    count += 1
                    errors += row['decode_error'] is not None
            if count >= next_progress:
                print('methods:', count, flush=True)
                next_progress = (count // 10000 + 1) * 10000
    receipt = {'schema': 'codepuri-filesystem-analysis-v1', 'apk_sha256': apk_hash,
               'inventory_sha256': digest(args.database), 'methods': count, 'decoded_bodies': bodies,
               'decode_errors': errors, 'components': len(components), 'java_roots': java_roots,
               'entries': entries, 'tools': tools,
               'gaps': ['Symbolic calls are not resolved runtime reflection/virtual dispatch.',
                        'Native assembly/symbols are evidence, not recovered native source or proven taint flow.',
                        'Server authorization and app execution require runtime/server evidence.',
                        'Legacy Java statuses are retained as historical inventory evidence; latest sources are separate snapshots.']}
    (args.out / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    (args.out / 'README.md').write_text('# Absher offline analysis filesystem\n\n'
        'Start with `components.json`, `resources/AndroidManifest.xml.tree.txt`, and `resources/table.txt`.\n'
        'Inspect current Java in the highest-numbered `java/` root; earlier roots provide full-export context.\n'
        'Use `methods.jsonl` to locate exact original DEX definitions, code-unit offsets and symbolic calls.\n'
        '`original/` preserves every APK entry, including assets, DEX, XML and native libraries.\n'
        '`native/` contains ELF details and disassembly. Decoder failures and analysis gaps are in `receipt.json`.\n'
        'This export does not turn missing Java/native/server behavior into recovered source.\n')
    db.close()
    print(json.dumps({name: receipt[name] for name in ['methods', 'decoded_bodies', 'decode_errors', 'components']}))


def extend_splits(args):
    path = args.out / 'receipt.json'
    receipt = json.loads(path.read_text())
    if receipt['apk_sha256'] != digest(args.apk):
        raise ValueError('base APK binding mismatch')
    splits = []
    for apk in args.split:
        prefix = PurePosixPath('splits') / safe_entry(apk.name)
        with zipfile.ZipFile(apk) as archive:
            names = archive.namelist()
            if any(name.endswith('.dex') for name in names):
                raise ValueError('split DEX requires a separate original-method inventory')
            if len(names) != len(set(names)):
                raise ValueError('duplicate split archive entries')
            for entry in archive.infolist():
                relative = safe_entry(entry.filename)
                if entry.is_dir():
                    continue
                dest = args.out / 'original' / prefix / relative
                dest.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(entry) as source, dest.open('xb') as output:
                    shutil.copyfileobj(source, output)
                receipt['entries'].append({'entry': str(prefix / relative), 'bytes': entry.file_size,
                                            'sha256': digest(dest), 'archive_sha256': digest(apk)})
                if entry.filename.endswith('.so'):
                    receipt['tools'].append(command(['readelf', '-aW', str(dest)], args.out / 'native' / prefix / (entry.filename + '.elf.txt')))
                    receipt['tools'].append(native_dump(dest, args.out / 'native' / prefix / (entry.filename + '.asm.txt')))
                if entry.filename.endswith('.xml'):
                    receipt['tools'].append(command([str(args.aapt2), 'dump', 'xmltree', str(apk), '--file', entry.filename], args.out / 'resources' / prefix / (entry.filename + '.tree.txt')))
        receipt['tools'].append(command([str(args.aapt2), 'dump', 'resources', str(apk)], args.out / 'resources' / prefix / 'table.txt'))
        splits.append({'archive': apk.name, 'sha256': digest(apk), 'entries': len(names)})
    receipt['splits'] = splits
    path.write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(splits, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--database', type=Path, required=True)
    parser.add_argument('--java', type=Path, action='append', default=[])
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--aapt2', type=Path, required=True)
    parser.add_argument('--split', type=Path, action='append', default=[])
    parser.add_argument('--extend-splits', action='store_true')
    parser.add_argument('--refresh-decoding', action='store_true')
    args = parser.parse_args()
    if args.refresh_decoding:
        refresh_decoding(args)
    elif args.extend_splits:
        extend_splits(args)
    else:
        build(args)
        if args.split:
            extend_splits(args)


if __name__ == '__main__':
    main()
