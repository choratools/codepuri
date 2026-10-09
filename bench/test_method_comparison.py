"""Tests for conservative original-identity attribution and outcome aggregation."""
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

from method_comparison import SCHEMA, ingest_jadx, observe, original_key, refresh_outcomes


class MethodComparisonTests(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        self.db.executescript(SCHEMA)
        self.db.execute("INSERT INTO apps VALUES(1,'fixture','Fixture','fixture.apk','hash')")
        self.db.execute("""INSERT INTO methods(id,app_id,entry,logical,pool,method_index,identity,owner,name,signature,flags,code_offset)
            VALUES(1,1,'classes.dex',0,0,9,'Lp/Raw;->run()I','Lp/Raw;','run','()I',1,256)""")

    def tearDown(self):
        self.db.close()

    def ingest(self, document):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / 'class.json').write_text(json.dumps(document))
            ingest_jadx(self.db, 1, path)

    def test_offset_and_raw_signature_survive_class_alias(self):
        self.ingest({'name': 'p.Renamed', 'dex': 'classes.dex', 'methods': [
            {'signature': 'run()I', 'offset': '0x100', 'lines': [{'code': 'return 7;'}]}]})
        self.assertEqual(self.db.execute('SELECT method_id,match_basis FROM observations').fetchone(),
                         (1, 'dex_code_offset_and_raw_signature'))

    def test_wrong_signature_at_correct_offset_is_not_attributed(self):
        self.ingest({'name': 'p.Renamed', 'dex': 'classes.dex', 'methods': [
            {'signature': 'run()J', 'offset': '0x100'}]})
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM observations').fetchone()[0], 0)
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM unmatched').fetchone()[0], 1)

    def test_exact_descriptor_does_not_hide_contradictory_code_offset(self):
        self.ingest({'name': 'p.Raw', 'dex': 'classes.dex', 'methods': [
            {'signature': 'run()I', 'offset': '0x999'}]})
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM observations').fetchone()[0], 0)

    def test_duplicate_code_item_identity_is_ambiguous(self):
        self.db.execute("""INSERT INTO methods(id,app_id,entry,logical,pool,method_index,identity,owner,name,signature,flags,code_offset)
            VALUES(2,1,'classes.dex',0,0,10,'Lp/Other;->run()I','Lp/Other;','run','()I',1,256)""")
        self.ingest({'name': 'p.Unknown', 'dex': 'classes.dex', 'methods': [
            {'signature': 'run()I', 'offset': '0x100'}]})
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM observations').fetchone()[0], 0)

    def test_inner_class_inherits_dex_and_failure_marker_stays_diagnostic(self):
        self.ingest({'name': 'p.Top', 'dex': 'classes.dex', 'methods': [], 'inner-classes': [
            {'name': 'p.Renamed', 'methods': [{'signature': 'run()I', 'offset': '0x100',
             'declaration': '/* Code decompiled incorrectly */', 'lines': []}]}]})
        self.assertEqual(self.db.execute('SELECT status FROM observations').fetchone()[0], 'diagnostic_marked')

    def test_emitted_dependency_diagnostic_does_not_mean_absent_body(self):
        observe(self.db, 1, 1, 'codepuri', 'body_emitted', 'exact', 'source.java', 1, semantic=True, dependency=False)
        observe(self.db, 1, 1, 'codepuri', 'explicitly_unavailable', 'exact', 'source.java', 2, reason='dependency unavailable')
        refresh_outcomes(self.db, 1)
        self.assertEqual(self.db.execute('SELECT codepuri_status FROM method_comparison').fetchone()[0], 'body_emitted_with_diagnostic')
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM method_comparison').fetchone()[0], 1)

    def test_rust_debug_key_preserves_descriptor_and_escaped_name(self):
        line = r'Some(MethodKey { owner: "Lp/Original;", name: "a\"b", parameters: ["I", "[Ljava/lang/String;"], return_type: "V" })'
        self.assertEqual(original_key(line), 'Lp/Original;->a"b(I[Ljava/lang/String;)V')


if __name__ == '__main__':
    unittest.main()
