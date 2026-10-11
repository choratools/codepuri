import tempfile
import unittest
from pathlib import Path
from filesystem_recovery import safe_entry, native_dump


class FilesystemRecoveryTests(unittest.TestCase):
    def test_archive_paths_cannot_escape_output(self):
        for name in ['../x', '/x', 'res/../../x', 'res\\..\\x']:
            with self.assertRaises(ValueError):
                safe_entry(name)
        self.assertEqual(str(safe_entry('res/xml/config.xml')), 'res/xml/config.xml')

    def test_protected_native_blob_is_not_reported_as_elf_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'protected.so'
            target = Path(directory)/'protected.asm.txt'
            payload = b'protected-container\x00\x01'
            source.write_bytes(payload)
            result = native_dump(source, target)
            self.assertEqual(source.read_bytes(), payload)
            self.assertEqual(result['format'], 'opaque-non-ELF')
            self.assertFalse(result['implementation_recovered'])
            self.assertIn(payload[:20].hex(), target.read_text())


if __name__ == '__main__':
    unittest.main()
