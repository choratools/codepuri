#!/usr/bin/env python3
"""Compare Codepuri source shape, separately from publication/semantic coverage.

Counts are source-shape observations, not a correctness or token-budget score.
Comments/literals are masked using the conservative LLM-pack lexer.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re

from llm_pack import compact_java, lexical_views


def scan(root):
    totals = Counter()
    files, methods, excluded = {}, {}, []
    for path in sorted(root.rglob('*.java')):
        relative = path.relative_to(root).as_posix()
        text = path.read_text()
        counts = Counter(files=1, source_bytes=len(text.encode()))
        for match in re.finditer(r'// original method pool index (\d+);([^\n]*)', text):
            flags = re.findall(r'(complete_body|semantic_complete|dependency_complete)=(true|false)', match[2])
            key = relative + ':' + match[1]
            if key in methods:
                raise ValueError(f'duplicate source identity: {key}')
            methods[key] = dict(flags)
        try:
            compact = compact_java(text)
            _, syntax, _ = lexical_views(text)
        except ValueError as error:
            counts['compact_bytes'] = counts['source_bytes']
            excluded.append({'path': relative, 'reason': str(error)})
        else:
            counts['compact_bytes'] = len(compact.encode())
            counts['syntax_nonblank_lines'] = sum(bool(line.strip()) for line in syntax.split('\n'))
            # Generated-local naming heuristics, including object/array locals.
            counts['generated_local_declarations'] = len(re.findall(
                r'\b[\w.$]+(?:\[\])*\s+(?:v\d+|edge\d+)\s*(?=[=;])', syntax))
            counts['phi_snapshot_declarations'] = len(re.findall(
                r'\b[\w.$]+(?:\[\])*\s+edge\d+\s*=', syntax))
            counts['pc_assignments'] = len(re.findall(r'\bdispatch\d+_pc\s*=', syntax))
            counts['if_nodes'] = len(re.findall(r'\bif\s*\(', syntax))
            counts['while_nodes'] = len(re.findall(r'\bwhile\s*\(', syntax))
            counts['for_nodes'] = len(re.findall(r'\bfor\s*\(', syntax))
            counts['short_circuit_operators'] = syntax.count('&&') + syntax.count('||')
            counts['conditional_operators'] = syntax.count('?')
            for match in re.finditer(r'\bswitch\s*\(dispatch\d+_pc\)\s*\{', syntax):
                start = match.end()
                depth, end = 1, start
                while depth and end < len(syntax):
                    depth += (syntax[end] == '{') - (syntax[end] == '}')
                    end += 1
                if depth:
                    raise ValueError(f'unbalanced generated dispatcher: {relative}')
                counts['dispatch_cases'] += len(re.findall(r'\bcase\s+\d+\s*:', syntax[start:end]))
        totals.update(counts)
        files[relative] = dict(counts)
    return dict(totals), files, methods, excluded


def scalar_report(path):
    if not path:
        return None
    return {name: int(value) for name, value in re.findall(
        r'^    ([a-z_]+): (\d+),$', path.read_text(), re.MULTILINE)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path)
    parser.add_argument('--after', type=Path)
    parser.add_argument('--before-log', type=Path)
    parser.add_argument('--after-log', type=Path)
    parser.add_argument('--test-log', type=Path)
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    result = {}
    if args.before or args.after:
        if not args.before or not args.after:
            parser.error('--before and --after must be provided together')
        before, before_files, before_methods, before_excluded = scan(args.before)
        after, after_files, after_methods, after_excluded = scan(args.after)
        result.update(before=before, after=after,
                      delta={key: after.get(key, 0) - before.get(key, 0) for key in before.keys() | after.keys()},
                      added_paths=sorted(after_files.keys() - before_files.keys()),
                      removed_paths=sorted(before_files.keys() - after_files.keys()),
                      added_method_identities=sorted(after_methods.keys() - before_methods.keys()),
                      removed_method_identities=sorted(before_methods.keys() - after_methods.keys()),
                      changed_publication_flags=[{'identity': key, 'before': before_methods[key], 'after': after_methods[key]}
                                                 for key in sorted(before_methods.keys() & after_methods.keys())
                                                 if before_methods[key] != after_methods[key]],
                      excluded_before=before_excluded, excluded_after=after_excluded)
        changes = []
        for key in before_files.keys() & after_files.keys():
            reduction = before_files[key]['compact_bytes'] - after_files[key]['compact_bytes']
            if reduction:
                changes.append({'path': key, 'compact_bytes_reduction': reduction})
        result['largest_compact_reductions'] = sorted(changes, key=lambda row: (-row['compact_bytes_reduction'], row['path']))[:20]
    result['before_report'] = scalar_report(args.before_log)
    result['after_report'] = scalar_report(args.after_log)
    if args.test_log:
        text = args.test_log.read_text()
        summaries = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
        result['test_summary'] = {'suites': len(summaries),
                                  'passed': sum(int(row[1]) for row in summaries),
                                  'failed': sum(int(row[2]) for row in summaries),
                                  'ignored': sum(int(row[3]) for row in summaries)}
        result['test_summary']['failed_suite_markers'] = text.count('test result: FAILED')
        result['test_summary']['note'] = 'Check process exit code independently; partial logs can contain passing suite summaries.'
    output = json.dumps(result, indent=2, sort_keys=True) + '\n'
    if args.out:
        args.out.write_text(output)
    print(output, end='')


if __name__ == '__main__':
    main()
