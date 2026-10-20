"""Pure recovery parsing/hash tests; no database or product acceptance."""
import hashlib
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
import sys

import run_core_recovery as recovery
from pg_fixture import FixtureError


class RecoveryContracts(unittest.TestCase):
    def test_inspection_requires_assistant_and_tool_market_fields(self):
        source=Path(__file__).resolve().parents[2]/'scripts/core-ops.py'
        spec=importlib.util.spec_from_file_location('core_ops_recovery_contract',source)
        module=importlib.util.module_from_spec(spec)
        sys.modules[spec.name]=module
        spec.loader.exec_module(module)
        value={name:False for name in module.CAPABILITIES}
        module.capabilities(value)
        self.assertIn('assistant',value)
        self.assertIn('tool_market',value)
        del value['assistant']
        with self.assertRaises(module.Refused):
            module.capabilities(value)

    def test_catalog_uses_core_length_prefix_not_line_hash(self):
        expected=hashlib.sha256(b'\0\0\0\0\0\0\0\x01a\0\0\0\0\0\0\0\x02bc').hexdigest()
        self.assertEqual(recovery.catalog_fingerprint(['a','bc']),expected)
        self.assertNotEqual(recovery.catalog_fingerprint(['ab','c']),expected)

    def test_utf8_prefix_counts_bytes(self):
        raw='测试'.encode()
        self.assertEqual(recovery.catalog_fingerprint(['测试']),
                         hashlib.sha256(len(raw).to_bytes(8,'big')+raw).hexdigest())

    def test_frozen_file_replacement_and_symlink_rejected(self):
        with tempfile.TemporaryDirectory(dir=os.environ['TMPDIR']) as directory:
            path=Path(directory)/'manifest.json'
            path.write_bytes(b'{}')
            expected=hashlib.sha256(b'{}').hexdigest()
            recovery.checked_file(path,expected)
            path.write_bytes(b'{ }')
            with self.assertRaises(FixtureError):
                recovery.checked_file(path,expected)
            link=Path(directory)/'link'
            link.symlink_to(path)
            with self.assertRaises(FixtureError):
                recovery.checked_file(link,hashlib.sha256(b'{ }').hexdigest())

    def test_frozen_file_requires_canonical_sha(self):
        with self.assertRaises(FixtureError):
            recovery.checked_file(Path('/never-opened'), 'x'*64)


if __name__=='__main__':
    unittest.main()
