import tempfile
import unittest
from pathlib import Path
from llm_pack import compact_java, compact_disassembly, member_source, page


class LlmPackTests(unittest.TestCase):
    def test_comments_removed_but_url_and_literal_comment_markers_preserved(self):
        source = '// diagnostic\nString url = "https://example.test/a//b"; // note\nchar slash = \'/\';\nString block = "/* literal */";\n/* real comment */\n'
        result = compact_java(source)
        self.assertNotIn('diagnostic', result)
        self.assertNotIn('real comment', result)
        self.assertIn('"https://example.test/a//b"', result)
        self.assertIn('"/* literal */"', result)

    def test_text_block_blank_lines_indentation_and_comment_text_stay_exact(self):
        source = 'String x = """\n    // literal\n\n    /* literal */\n    """;\n// remove\n'
        result = compact_java(source)
        self.assertIn('"""\n    // literal\n\n    /* literal */\n    """', result)
        self.assertNotIn('// remove', result)

    def test_unicode_separators_in_literals_do_not_change_source_line_count(self):
        source = 'String x="a\u2028b\u2029c"; // remove\nreturn;\n'
        result = compact_java(source)
        self.assertIn('"a\u2028b\u2029c"', result)
        self.assertEqual(result.count('\n'), 2)

    def test_stale_identity_locator_does_not_extract_another_member(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'class.java'
            path.write_text('// removed identity\npublic void unrelated() {}\n')
            with self.assertRaises(ValueError):
                member_source(path, 1, 'Lp/C;->wanted()V')

    def test_member_extraction_ignores_braces_in_strings_and_annotations(self):
        source = '// identity\n@Ann(values={"}"})\npublic void run() { String x="}"; /* } */ if(true) { return; } }\npublic void next() {}\n'
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'class.java'
            path.write_text(source)
            result = member_source(path, 1)
        self.assertIn('public void run()', result)
        self.assertNotIn('public void next()', result)
        self.assertIn('String x="}"', result)

    def test_unicode_preprocessing_is_not_silently_rewritten(self):
        with self.assertRaises(ValueError):
            compact_java(r'\u002f\u002f comment')

    def test_paging_marks_incomplete_and_preserves_whole_lines(self):
        fragment, metadata = page('line one\nline two\n', 0, 10)
        self.assertEqual(fragment, 'line one\n')
        self.assertFalse(metadata['complete'])
        self.assertEqual(metadata['next_line'], 1)

    def test_compact_dex_retains_offsets_targets_literals_and_symbols(self):
        original = '00000a: invoke-static registers=[1, 2] literal=None pool=Some(3) reference=Some("method:Lp/C;->run(II)V") branch_target=None payload=None\n00000d: if-eqz registers=[1] literal=Some(7) pool=None reference=None branch_target=Some(15) payload=None\n'
        result = compact_disassembly(original)
        self.assertIn('00000a: invoke-static [1,2] method:Lp/C;->run(II)V', result)
        self.assertIn('#7 ->0xf', result)
        self.assertNotIn('literal=None', result)


if __name__ == '__main__':
    unittest.main()
