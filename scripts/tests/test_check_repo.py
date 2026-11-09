"""Regression coverage for staged snapshots and repository constraints."""

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / 'check_repo.py'
SPEC = importlib.util.spec_from_file_location('check_repo', SCRIPT)
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class ContentChecks(unittest.TestCase):
    def test_valid_relative_link_with_encoded_path(self):
        files = {
            'docs/readme.md': b'[Details](../file%20name.md#heading)\n',
            'file name.md': b'# Heading\n',
        }
        self.assertEqual([], CHECKER.check_files(files))

    def test_broken_link(self):
        errors = CHECKER.check_files({'README.md': b'[Missing](missing.md)\n'})
        self.assertTrue(any('missing linked file' in error for error in errors))

    def test_link_outside_repository(self):
        errors = CHECKER.check_files({'README.md': b'[Outside](../outside.md)\n'})
        self.assertTrue(any('leaves the repository' in error for error in errors))

    def test_text_constraints(self):
        files = {
            'crlf.md': b'line\r\n',
            'spaces.md': b'line \n',
            'newline.md': b'line',
            'encoding.md': b'\xff\n',
            'bom.md': b'\xef\xbb\xbftext\n',
        }
        self.assertEqual(5, len(CHECKER.check_files(files)))

    def test_batch_crlf_and_binary_are_supported(self):
        files = {'setup.cmd': b'@echo off\r\n', 'icon.png': b'\xff\x00'}
        self.assertEqual([], CHECKER.check_files(files))

    def test_private_files_and_environment_examples(self):
        files = {'.env': b'', '.env.example': b'', 'private.key': b''}
        self.assertEqual(2, len(CHECKER.check_files(files)))

    def test_large_artifact(self):
        files = {'package.zip': b'0' * (CHECKER.MAX_FILE_BYTES + 1)}
        self.assertTrue(any('10 MiB' in error for error in CHECKER.check_files(files)))


class SnapshotChecks(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='wccs-check-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git('init', '-q')
        self.git('config', 'core.autocrlf', 'false')

    def git(self, *arguments):
        subprocess.run(
            ['git', '-C', str(self.root), *arguments],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def write(self, name, data):
        (self.root / name).write_bytes(data)

    def test_unstaged_fix_does_not_hide_staged_error(self):
        self.write('README.md', b'bad \n')
        self.git('add', 'README.md')
        self.write('README.md', b'fixed\n')
        self.assertEqual([], CHECKER.check_files(CHECKER.snapshot(self.root, False)))
        self.assertTrue(CHECKER.check_files(CHECKER.snapshot(self.root, True)))

    def test_untracked_link_target_does_not_satisfy_index(self):
        self.write('README.md', b'[Details](details.md)\n')
        self.git('add', 'README.md')
        self.write('details.md', b'# Details\n')
        self.assertEqual([], CHECKER.check_files(CHECKER.snapshot(self.root, False)))
        self.assertTrue(CHECKER.check_files(CHECKER.snapshot(self.root, True)))

    def test_staged_deletion_removes_link_target(self):
        self.write('README.md', b'[Details](details.md)\n')
        self.write('details.md', b'# Details\n')
        self.git('add', 'README.md', 'details.md')
        self.git('rm', '--cached', 'details.md')
        self.assertTrue(CHECKER.check_files(CHECKER.snapshot(self.root, True)))

    def test_ignored_files_are_excluded(self):
        self.write('.gitignore', b'local/\n')
        (self.root / 'local').mkdir()
        self.write('local/private.key', b'local fixture\n')
        files = CHECKER.snapshot(self.root, False)
        self.assertNotIn('local/private.key', files)

    def test_large_file_is_rejected_before_snapshot_load(self):
        self.write('large.zip', b'0' * (CHECKER.MAX_FILE_BYTES + 1))
        with self.assertRaisesRegex(ValueError, '10 MiB'):
            CHECKER.snapshot(self.root, False)
        self.git('add', 'large.zip')
        with self.assertRaisesRegex(ValueError, '10 MiB'):
            CHECKER.snapshot(self.root, True)

    def test_actual_pre_commit_rejects_bad_index_and_accepts_fix(self):
        (self.root / 'scripts').mkdir()
        (self.root / '.githooks').mkdir()
        self.write('scripts/check_repo.py', SCRIPT.read_bytes())
        hook = SCRIPT.parents[1] / '.githooks' / 'pre-commit'
        self.write('.githooks/pre-commit', hook.read_bytes())
        self.write('README.md', b'bad \n')
        self.git('config', 'core.hooksPath', '.githooks')
        self.git('add', 'scripts/check_repo.py', 'README.md')
        self.git('add', '--chmod=+x', '.githooks/pre-commit')
        command = [
            'git', '-C', str(self.root),
            '-c', 'user.name=WCCS Test',
            '-c', 'user.email=test@example.invalid',
            '-c', 'commit.gpgsign=false',
            'commit', '-m', 'test(repo): verify hook',
        ]
        rejected = subprocess.run(command, capture_output=True)
        self.assertNotEqual(0, rejected.returncode)
        self.assertIn(b'trailing whitespace', rejected.stderr)
        self.write('README.md', b'fixed\n')
        self.git('add', 'README.md')
        accepted = subprocess.run(command, capture_output=True)
        self.assertEqual(0, accepted.returncode, accepted.stderr.decode())


if __name__ == '__main__':
    unittest.main()
