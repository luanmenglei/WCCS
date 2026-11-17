import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile


SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("client_package", SCRIPTS / "package-client.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class ClientPackageTests(unittest.TestCase):
    def test_image_delivery_rejects_private_target_material(self):
        image_spec = importlib.util.spec_from_file_location("image_package", SCRIPTS / "package-image.py")
        image_package = importlib.util.module_from_spec(image_spec)
        image_spec.loader.exec_module(image_package)
        with tempfile.TemporaryDirectory(prefix="wccs-image-package-") as temporary:
            root = Path(temporary)
            names = ["bin/wccs-agent.exe", "bin/wccs-session.exe", "bin/wccs-inventory.exe", "bin/wccs-login-provider.dll", "runtime/pwsh.exe"]
            names += ["scripts/" + name for name in package.SCRIPTS]
            source = root / "client.zip"
            package.write_archive(source, {name: b"synthetic" for name in names}, {
                "schema_version": 1, "state_schema": 1, "target": "x86_64-pc-windows-msvc", "version": "test",
            })
            ca = root / "ca.crt"
            ca.write_bytes(b"synthetic CA")
            target = {"machine_name": "503-41", "machine_guid": "00000000-0000-0000-0000-000000000001",
                      "install_directory": "C:/Program Files/WCCS", "service_name": "WCCS-Agent",
                      "server": "https://example.test", "ca_sha256": package.digest(ca.read_bytes())}
            output = root / "image.zip"
            image_package.build(source, ca, target, output)
            with zipfile.ZipFile(output) as archive:
                self.assertEqual(json.loads(archive.read("image/image.json"))["server"], target["server"])
                self.assertNotIn("image/mother-target.json", archive.namelist())
                self.assertNotIn("device.json", archive.namelist())
                self.assertIn("M3-M5-FIELD-GUIDE.md", archive.namelist())
                contest = json.loads(archive.read("contest-scripts-v1.json"))
                self.assertEqual(contest["runtime_file_version"], "7.6.9.500")
                self.assertEqual({item["path"] for item in contest["files"]},
                                 {"compilecpp.ps1", "submit.ps1", "contest-common.ps1"})
            with self.assertRaises(ValueError):
                image_package.build(source, ca, target | {"enrollment_token": "synthetic-secret"}, root / "bad.zip")

    @unittest.skipUnless(os.name == "nt", "Windows image routines")
    def test_image_identity_independence_retry_and_seal(self):
        with tempfile.TemporaryDirectory(prefix="wccs-image-") as temporary:
            result = subprocess.run(
                ["pwsh", "-NoProfile", "-File", str(SCRIPTS / "tests/image-common-tests.ps1"), "-Root", temporary],
                text=True, capture_output=True, timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("image_common_checks_passed", result.stdout)

    def test_deterministic_archive_and_exact_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = {"bin/a.exe": b"synthetic binary", "scripts/a.ps1": b"Write-Output 'fixture'\r\n"}
            first = root / "first.zip"
            second = root / "second.zip"
            package.write_archive(first, files, {"version": "test"})
            package.write_archive(second, dict(reversed(list(files.items()))), {"version": "test"})
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with zipfile.ZipFile(first) as archive:
                manifest = json.loads(archive.read("manifest.json"))
                for item in manifest["files"]:
                    data = archive.read(item["path"])
                    self.assertEqual(item["sha256"], hashlib.sha256(data).hexdigest())
                    self.assertEqual(item["size"], len(data))
            with self.assertRaises(FileExistsError):
                package.write_archive(first, files, {})

    def test_archive_refuses_windows_aliases_and_traversal(self):
        for name in ("../escape", "C:/escape", "/escape", "bin/../x", "bin/.. /x", "bin/x.", "CON.txt", "bin//x"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(ValueError):
                    package.write_archive(Path(temporary) / "bad.zip", {name: b"x"}, {})
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                package.write_archive(Path(temporary) / "bad.zip", {"bin/a": b"x", "BIN/A": b"y"}, {})

    @unittest.skipUnless(os.name == "nt", "Windows filesystem transaction coverage")
    def test_powershell_package_identity_rollback_and_redaction(self):
        with tempfile.TemporaryDirectory(prefix="wccs-field-") as temporary:
            result = subprocess.run(
                ["pwsh", "-NoProfile", "-File", str(SCRIPTS / "tests/client-common-tests.ps1"), "-Root", temporary],
                text=True, capture_output=True, timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("client_common_checks_passed", result.stdout)

    @unittest.skipUnless(os.name == "nt", "Windows entry point rejection coverage")
    def test_entry_points_refuse_wrong_target_and_parameters(self):
        with tempfile.TemporaryDirectory(prefix="wccs-field-entry-") as temporary:
            root = Path(temporary)
            files = {"scripts/" + name: (SCRIPTS / name).read_bytes() for name in package.SCRIPTS}
            for name in ("wccs-agent.exe", "wccs-session.exe", "wccs-inventory.exe", "wccs-login-provider.dll"):
                files["bin/" + name] = b"synthetic non-executable fixture"
            files["runtime/pwsh.exe"] = b"synthetic non-executable fixture"
            archive = root / "fixture.zip"
            package.write_archive(archive, files, {"schema_version": 1, "state_schema": 1,
                "version": "fixture", "target": "x86_64-pc-windows-msvc", "windows_minimum_build": 15063})
            expanded = root / "package"
            with zipfile.ZipFile(archive) as zipped:
                zipped.extractall(expanded)
            command = ["pwsh", "-NoProfile", "-File", str(expanded / "scripts/client-field.ps1")]
            for _ in range(2):
                result = subprocess.run(command + ["-Action", "Verify"], text=True, capture_output=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            target = root / "target.json"
            target.write_text(json.dumps({"machine_name": "INTENTIONALLY-WRONG", "machine_guid": "00000000-0000-0000-0000-000000000000"}))
            for action in ("Preflight", "Install", "Start", "Recover", "Rollback", "RollbackInstall", "InitializeLogin"):
                result = subprocess.run(command + ["-Action", action, "-Target", str(target)], text=True, capture_output=True, timeout=30)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("target_identity_mismatch", result.stderr)
            result = subprocess.run(command + ["-Action", "InvalidAction"], text=True, capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            result = subprocess.run(command + ["-Action", "Install"], text=True, capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("explicit_target_file_required", result.stderr)
            self.assertEqual(list(root.glob("**/field-installation.json")), [])

    @unittest.skipUnless(os.name == "nt", "Windows single-machine handoff coverage")
    def test_single_machine_launcher_checks_before_mutations(self):
        with tempfile.TemporaryDirectory(prefix="wccs field 中文-") as temporary:
            root = Path(temporary)
            client = root / "client"
            files = {"scripts/" + name: (SCRIPTS / name).read_bytes() for name in package.SCRIPTS}
            for name in ("wccs-agent.exe", "wccs-session.exe", "wccs-inventory.exe", "wccs-login-provider.dll"):
                files["bin/" + name] = b"synthetic non-executable fixture"
            files["runtime/pwsh.exe"] = b"synthetic non-executable fixture"
            archive = root / "fixture.zip"
            package.write_archive(archive, files, {"schema_version": 1, "state_schema": 1,
                "version": "fixture", "target": "x86_64-pc-windows-msvc", "windows_minimum_build": 15063})
            with zipfile.ZipFile(archive) as zipped:
                zipped.extractall(client)
            (root / "inputs").mkdir()
            target = root / "inputs/target.json"
            target.write_text(json.dumps({"machine_name": "INTENTIONALLY-WRONG",
                "machine_guid": "00000000-0000-0000-0000-000000000000"}))
            (root / "field-test.ps1").write_bytes((SCRIPTS / "field-test.ps1").read_bytes())
            names = ("field-test.ps1", "inputs/target.json", "client/manifest.json")
            manifest = {"files": [{"path": name, "sha256": hashlib.sha256((root / name).read_bytes()).hexdigest()}
                                  for name in names]}
            (root / "bundle-manifest.json").write_text(json.dumps(manifest))
            command = ["pwsh", "-NoProfile", "-File", str(root / "field-test.ps1")]
            for _ in range(2):
                result = subprocess.run(command + ["-Action", "Verify"], capture_output=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stderr)
            for action in ("Install", "PrepareLogin", "InitializeLogin", "Start", "Recover", "RemoveProvider", "RollbackInstall", "Diagnose"):
                result = subprocess.run(command + ["-Action", action], capture_output=True, timeout=30)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b"target_identity_mismatch", result.stderr)
            for arguments in (("-Action", "Invalid"), ("-Action", "Install", "-ForceStop")):
                result = subprocess.run(command + list(arguments), capture_output=True, timeout=30)
                self.assertNotEqual(result.returncode, 0)
            target.write_text(target.read_text() + "\n")
            result = subprocess.run(command + ["-Action", "Verify"], capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"bundle_hash_mismatch", result.stderr)
            self.assertFalse((root / "work").exists())
            self.assertFalse((root / "evidence").exists())


if __name__ == "__main__":
    unittest.main()
