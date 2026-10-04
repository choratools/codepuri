#!/usr/bin/env python3
"""Summarize original method identities without counting fallback declarations."""
import argparse
import collections
import decimal
import hashlib
import json
import pathlib
import re


def load(path):
    rows = {}
    for line in path.read_text().splitlines():
        row = json.loads(line)
        if row['identity'] in rows:
            raise ValueError('duplicate original identity: ' + row['identity'])
        rows[row['identity']] = row
    return rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('ledger', type=pathlib.Path)
    parser.add_argument('--baseline', type=pathlib.Path)
    parser.add_argument('--target-percent', type=decimal.Decimal, default=decimal.Decimal(100))
    parser.add_argument('--require-no-regressions', action='store_true',
                        help='Save the identity delta, then fail if any accepted original was lost.')
    parser.add_argument('--output', required=True, type=pathlib.Path)
    args = parser.parse_args()
    if args.require_no_regressions and not args.baseline:
        parser.error('--require-no-regressions requires --baseline')
    if not args.target_percent.is_finite() or not 0 < args.target_percent <= 100:
        parser.error('--target-percent must be finite and in (0, 100]')
    rows = load(args.ledger)
    if len(rows) != 205497:
        raise ValueError('original definition denominator differs from 205497')
    statuses = collections.Counter(r['status'] for r in rows.values())
    accepted_statuses = {'complete_body', 'proven_enum_equivalence'}
    accepted = {k for k, r in rows.items() if r['status'] in accepted_statuses}
    required = int((decimal.Decimal(len(rows)) * args.target_percent / 100)
                   .to_integral_value(rounding=decimal.ROUND_CEILING))
    code = {k for k, r in rows.items() if r['has_code']}
    required_code = int((decimal.Decimal(len(code)) * args.target_percent / 100)
                        .to_integral_value(rounding=decimal.ROUND_CEILING))
    genuine_declarations = {k for k, r in rows.items()
                            if not r['has_code'] and r['status'] == 'declaration_only'
                            and r['access_flags'] & (0x100 | 0x400)}
    reasons = collections.Counter()
    for row in rows.values():
        if 'reason' in row:
            reason = re.sub(r'(ValueId|BlockId|InsnId|EdgeId)\([^)]*\)', '<id>', row['reason'])
            reasons[reason] += 1
    result = dict(denominator=len(rows), target_percent=float(args.target_percent),
                  required_definitions=required,
                  strict_accepted=len(accepted), strict_percent=100 * len(accepted) / len(rows),
                  code_bearing_definitions=len(code),
                  required_code_bearing_definitions=required_code,
                  strict_code_bearing_accepted=len(accepted & code),
                  strict_code_bearing_percent=100 * len(accepted & code) / len(code),
                  remaining_code_bearing_to_target=max(0, required_code-len(accepted & code)),
                  genuine_bodyless_declarations=len(genuine_declarations),
                  original_form_accepted=len(accepted | genuine_declarations),
                  original_form_percent=100 * len(accepted | genuine_declarations) / len(rows),
                  remaining_original_forms_to_target=max(0, required-len(accepted | genuine_declarations)),
                  remaining_to_target=max(0, required-len(accepted)), statuses=dict(statuses),
                  ledger=str(args.ledger), ledger_sha256=hashlib.sha256(args.ledger.read_bytes()).hexdigest(),
                  rejection_reasons=reasons.most_common())
    if args.target_percent == 90:
        result['required_90_percent'] = required
    if args.baseline:
        old = load(args.baseline)
        if set(old) != set(rows):
            raise ValueError('original identity set changed')
        for identity in rows:
            if any(old[identity][key] != rows[identity][key]
                   for key in ('has_code', 'access_flags', 'category')):
                raise ValueError('original definition metadata changed: ' + identity)
        previous = {k for k, r in old.items() if r['status'] in accepted_statuses}
        result['gained'] = sorted(accepted-previous)
        result['lost'] = sorted(previous-accepted)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k: v for k, v in result.items() if k not in {'gained', 'lost', 'rejection_reasons'}}, indent=2))
    if args.require_no_regressions and result['lost']:
        raise SystemExit(f"accepted original identities lost: {len(result['lost'])}; "
                         f"see {args.output}")


if __name__ == '__main__':
    main()
