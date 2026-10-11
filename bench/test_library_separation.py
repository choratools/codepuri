import argparse
import unittest

from library_separation import preview, score


class LibrarySeparationTests(unittest.TestCase):
    def test_threshold_does_not_bypass_implementation_or_anchor_checks(self):
        app = {'Llib/Exact;': {'implementation_sha256': 'same-body', 'anchors': ['unique']},
               'Llib/Weak;': {'implementation_sha256': 'same-trivial', 'anchors': []},
               'Llib/Changed;': {'implementation_sha256': 'changed', 'anchors': ['other']},
               'Lapp/ExactNeighbor;': {'implementation_sha256': 'neighbor', 'anchors': ['neighbor']}}
        reference = {'Llib/Exact;': {'implementation_sha256': 'same-body', 'anchors': ['unique']},
                     'Llib/Weak;': {'implementation_sha256': 'same-trivial', 'anchors': []},
                     'Llib/Changed;': {'implementation_sha256': 'original', 'anchors': ['other']}}
        self.assertFalse(any(row['selected'] for row in preview(app, reference, 0.93, 0.95)))
        accepted = preview(app, reference, 0.93, 0.90)
        self.assertEqual([row['owner'] for row in accepted if row['selected']], ['Llib/Exact;'])
        self.assertFalse(any(row['owner'] == 'Lapp/ExactNeighbor;' for row in accepted))
        app['Lapp/Clone;'] = {'implementation_sha256': 'clone', 'anchors': ['unique']}
        self.assertFalse(any(row['selected'] for row in preview(app, reference, 1.0, 0.0)))

    def test_missing_fingerprints_are_not_a_match(self):
        app = {'Llib/Unsupported;': {'implementation_sha256': None, 'anchors': ['unique']}}
        self.assertFalse(preview(app, app, 1.0, 0.0)[0]['selected'])

    def test_score_range_is_finite(self):
        for text in ['NaN', 'inf', '-0.1', '1.1']:
            with self.assertRaises(argparse.ArgumentTypeError):
                score(text)
        self.assertEqual(score('0.95'), 0.95)


if __name__ == '__main__':
    unittest.main()
