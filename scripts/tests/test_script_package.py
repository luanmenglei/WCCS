import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "script_package", Path(__file__).resolve().parents[1] / "package-contest-scripts.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class ScriptPackageTests(unittest.TestCase):
    def build(self, root):
        return package.build(root, "contest", "v1", "config1", "powershell",
                             "7.5.0.0", "project synthetic fixture", "MIT")

    def test_preserves_script_bytes_and_records_exact_digest(self):
        with tempfile.TemporaryDirectory() as root:
            content = b"Write-Output 'synthetic'\r\n"
            Path(root, "compilecpp.ps1").write_bytes(content)
            result = self.build(root)
            self.assertEqual(result["files"][0]["content"].encode(), content)
            self.assertEqual(result["files"][0]["sha256"], hashlib.sha256(content).hexdigest())

    def test_rejects_reserved_manifest_case_alias_and_output_limit(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root, "MANIFEST.json")
            path.write_text("{}")
            with self.assertRaises(ValueError):
                self.build(root)
            path.unlink()
            Path(root, "large.ps1").write_bytes(b"a" * (128 * 1024 + 1))
            with self.assertRaises(ValueError):
                self.build(root)
