import json
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SOURCE = Path(__file__).resolve().parents[1] / "contest"
MOCK_HTTP = r'''
Add-Type -TypeDefinition @'
using System;
using System.Net;
using System.Net.Http;
using System.Threading;
using System.Threading.Tasks;
public sealed class ContestTestHandler : HttpMessageHandler {
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken token) {
        if (request.RequestUri.AbsolutePath.EndsWith("/user"))
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) {
                Content = new StringContent("{\"username\":\"synthetic\",\"team_id\":\"team1\",\"roles\":[\"team\"]}") });
        if (request.Method == HttpMethod.Get)
            return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) {
                Content = new StringContent("[{\"id\":\"hello\",\"label\":\"A\"}]") });
        string mode = Environment.GetEnvironmentVariable("WCCS_CONTEST_TEST_RESPONSE");
        if (mode == "lost")
            return Task.FromException<HttpResponseMessage>(new HttpRequestException("synthetic response loss"));
        var status = mode == "reject" ? HttpStatusCode.Forbidden :
            mode == "server" ? HttpStatusCode.InternalServerError : HttpStatusCode.OK;
        return Task.FromResult(new HttpResponseMessage(status) {
            Content = new StringContent("{\"id\":\"123\"}") });
    }
}
'@
function New-ContestHttpClient { return [Net.Http.HttpClient]::new([ContestTestHandler]::new()) }
'''


@unittest.skipUnless(shutil.which("pwsh"), "PowerShell 7 required")
class ContestScriptsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wccs-contest-tests-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "中文 空格"
        self.root.mkdir()
        self.scripts = self.root / "scripts"
        shutil.copytree(SOURCE, self.scripts)
        self.config = self.root / "contest.json"
        self.config.write_text(json.dumps({
            "schema_version": 1, "version": "test-1",
            "compiler_path": shutil.which("g++") or "missing",
            "compiler_arguments": ["-std=c++17", "-O2"],
            "judge_url": "https://judge.example.test", "contest_id": "demo", "language_id": "cpp",
        }), encoding="utf-8")
        self.source = self.root / "main.cpp"
        self.source.write_text('#include <cstdio>\nint main(){puts("ok");}\n', encoding="utf-8")

    def run_script(self, name, *args, response="success"):
        return subprocess.run(
            ["pwsh", "-NoProfile", "-File", str(self.scripts / name),
             "-Configuration", str(self.config), *args],
            cwd=self.root, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60,
            env=os.environ | {"WCCS_CONTEST_TEST_RESPONSE": response},
        )

    @unittest.skipUnless(shutil.which("g++"), "GNU C++ required")
    def test_compile_paths_and_failure_remove_stale_output(self):
        output = self.root / "结果 with spaces.exe"
        result = self.run_script("compilecpp.ps1", str(self.source), "-o", str(output))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(subprocess.check_output([str(output)], text=True).strip(), "ok")
        self.source.write_text("this cannot compile", encoding="utf-8")
        result = self.run_script("compilecpp.ps1", str(self.source), "-o", str(output))
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertFalse(output.exists())
        self.assertFalse(list(self.root.glob(".wccs-*.exe")))

    def test_submit_records_acceptance_rejection_and_uncertainty(self):
        common = self.scripts / "contest-common.ps1"
        common.write_text(common.read_text(encoding="utf-8") + MOCK_HTTP, encoding="utf-8")
        for response, expected_exit, state in [
            ("success", 0, "accepted"), ("reject", 3, "rejected"),
            ("lost", 4, "unknown"), ("server", 4, "unknown"),
        ]:
            with self.subTest(response=response):
                before = set((self.root / ".wccs-submissions").glob("*.json"))
                result = self.run_script("submit.ps1", "A", str(self.source), "-Yes", "-Resubmit", response=response)
                self.assertEqual(result.returncode, expected_exit, result.stderr)
                created = set((self.root / ".wccs-submissions").glob("*.json")) - before
                self.assertEqual(len(created), 1)
                receipt = json.loads(created.pop().read_text(encoding="utf-8-sig"))
                self.assertEqual(receipt["state"], state)
                self.assertEqual(len(receipt["files"][0]["sha256"]), 64)
                self.assertNotIn("password", receipt)
                if state in ("accepted", "unknown"):
                    repeated = self.run_script("submit.ps1", "A", str(self.source), "-Yes", response="lost")
                    self.assertEqual(repeated.returncode, 0 if state == "accepted" else 4)
                    self.assertEqual(len(list((self.root / ".wccs-submissions").glob("*.json"))), len(before) + 1)

    def test_launcher_submits_two_sources_in_order(self):
        common = self.scripts / "contest-common.ps1"
        common.write_text(common.read_text(encoding="utf-8") + MOCK_HTTP, encoding="utf-8")
        helper = self.root / "helper.cpp"
        helper.write_text("int helper(){return 1;}", encoding="utf-8")
        launcher = self.root / "contest-launch.ps1"
        shutil.copy2(SOURCE.parent / launcher.name, launcher)
        config = self.scripts / "contest.json"
        shutil.copy2(self.config, config)
        def digest(path):
            return hashlib.sha256(path.read_bytes()).hexdigest()
        manifest = self.scripts / "manifest.json"
        manifest.write_text(json.dumps({"files": [
            {"path": p.name, "sha256": digest(p)} for p in self.scripts.glob("*.ps1")
        ]}), encoding="utf-8")
        (self.root / "active.json").write_text(json.dumps({
            "directory": str(self.scripts), "manifest_sha256": digest(manifest),
            "configuration_sha256": digest(config),
        }), encoding="utf-8")
        subprocess.run(["pwsh", "-NoProfile", "-File", str(launcher), "submit", "A",
                        str(self.source), str(helper), "-Language", "cpp", "-Yes"],
                       cwd=self.root, check=True, capture_output=True, timeout=60)
        receipt = json.loads(next((self.root / ".wccs-submissions").glob("*.json")).read_text(encoding="utf-8-sig"))
        self.assertEqual(receipt["files"], [{"name": p.name, "sha256": digest(p)} for p in (self.source, helper)])

    def test_submit_checks_domjudge_filename_before_post(self):
        common = self.scripts / "contest-common.ps1"
        common.write_text(common.read_text(encoding="utf-8") + MOCK_HTTP, encoding="utf-8")
        invalid = self.root / "中文.cpp"
        invalid.write_bytes(self.source.read_bytes())
        result = self.run_script("submit.ps1", "A", str(invalid), "-Yes")
        self.assertEqual(result.returncode, 3)
        self.assertIn("DOMjudge", result.stderr)
        self.assertFalse((self.root / ".wccs-submissions").exists())

    @unittest.skipUnless(shutil.which("g++"), "GNU C++ required")
    def test_activation_launcher_preserves_arguments_and_rejects_tampering(self):
        launcher = self.root / "contest-launch.ps1"
        shutil.copy2(SOURCE.parent / launcher.name, launcher)
        configuration = self.scripts / "contest.json"
        shutil.copy2(self.config, configuration)
        manifest = self.scripts / "manifest.json"
        manifest.write_text(json.dumps({"files": [
            {"path": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for path in self.scripts.glob("*.ps1")
        ]}), encoding="utf-8")
        (self.root / "active.json").write_text(json.dumps({
            "directory": str(self.scripts),
            "manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
            "configuration_sha256": hashlib.sha256(configuration.read_bytes()).hexdigest(),
        }), encoding="utf-8")
        output = self.root / "chosen output.exe"
        command = ["pwsh", "-NoProfile", "-File", str(launcher), "compilecpp", str(self.source), "-o", str(output)]
        result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(output.is_file())
        configuration.write_text("{}", encoding="utf-8")
        result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("校验失败", result.stderr)


if __name__ == "__main__":
    unittest.main()
