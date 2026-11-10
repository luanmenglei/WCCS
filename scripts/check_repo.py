"""Check repository files or the exact Git index using the standard library."""

from __future__ import annotations

import argparse
import posixpath
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath
from urllib.parse import unquote, urlsplit


TEXT_SUFFIXES = {
    '.md', '.py', '.ps1', '.sh', '.toml', '.json', '.yaml', '.yml',
    '.cs', '.csproj', '.props', '.targets', '.sln', '.slnx', '.ts',
    '.tsx', '.js', '.jsx', '.css', '.html', '.xml', '.txt', '.svg',
    '.bat', '.cmd', '.sql', '.config', '.lock', '.rs',
}
TEXT_NAMES = {'.editorconfig', '.gitattributes', '.gitignore', 'pre-commit'}
PRIVATE_SUFFIXES = {'.key', '.pem', '.pfx', '.p12'}
MAX_FILE_BYTES = 10 * 1024 * 1024
MARKDOWN_LINK = re.compile(r'!?\[[^\]\n]*\]\(<?([^\s)>]+)>?(?:\s+"[^"]*")?\)')


def git(root: Path, *arguments: str) -> bytes:
    result = subprocess.run(
        ['git', '-C', str(root), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout


def snapshot(root: Path, staged: bool) -> dict[str, bytes]:
    arguments = ['ls-files', '--cached', '-z']
    if not staged:
        arguments.extend(['--others', '--exclude-standard'])
    names = git(root, *arguments).decode('utf-8').split('\0')
    files = {}
    for name in sorted(set(names) - {''}):
        path = root / name
        if staged:
            size = int(git(root, 'cat-file', '-s', f':{name}'))
            if size > MAX_FILE_BYTES:
                raise ValueError(f'{name}: exceeds the 10 MiB repository file limit')
            files[name] = git(root, 'show', f':{name}')
        elif path.is_file():
            with path.open('rb') as stream:
                data = stream.read(MAX_FILE_BYTES + 1)
            if len(data) > MAX_FILE_BYTES:
                raise ValueError(f'{name}: exceeds the 10 MiB repository file limit')
            files[name] = data
    return files


def check_links(name: str, text: str, names: set[str]) -> list[str]:
    errors = []
    for match in MARKDOWN_LINK.finditer(text):
        target = match.group(1)
        parsed = urlsplit(target)
        if parsed.scheme or parsed.netloc or not parsed.path:
            continue
        path = posixpath.normpath(
            posixpath.join(posixpath.dirname(name), unquote(parsed.path))
        )
        if path.startswith('../') or path.startswith('/'):
            errors.append(f'{name}: link leaves the repository: {target}')
        elif path not in names:
            errors.append(f'{name}: missing linked file: {target}')
    return errors


def check_files(files: dict[str, bytes]) -> list[str]:
    errors = []
    names = set(files)
    for name, data in files.items():
        path = PurePosixPath(name)
        suffix = path.suffix.lower()
        basename = path.name.lower()
        is_environment = basename == '.env' or basename.startswith('.env.')
        if suffix in PRIVATE_SUFFIXES or (
            is_environment and not basename.endswith('.example')
        ):
            errors.append(f'{name}: credential file must stay outside Git')
        if len(data) > MAX_FILE_BYTES:
            errors.append(f'{name}: exceeds the 10 MiB repository file limit')
        if suffix not in TEXT_SUFFIXES and path.name not in TEXT_NAMES:
            continue
        try:
            text = data.decode('utf-8')
        except UnicodeDecodeError:
            errors.append(f'{name}: expected UTF-8 text')
            continue
        if text.startswith('\ufeff'):
            errors.append(f'{name}: remove the UTF-8 byte order mark')
        if suffix in {'.bat', '.cmd'}:
            text = text.replace('\r\n', '\n')
        if '\r' in text:
            errors.append(f'{name}: expected LF line endings')
        if text and not text.endswith('\n'):
            errors.append(f'{name}: missing final newline')
        for number, line in enumerate(text.splitlines(), start=1):
            if line.rstrip(' \t') != line:
                errors.append(f'{name}:{number}: trailing whitespace')
        if suffix == '.md':
            errors.extend(check_links(name, text, names))
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--staged', action='store_true', help='check the Git index')
    arguments = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        files = snapshot(root, arguments.staged)
        errors = check_files(files)
    except (OSError, UnicodeError, ValueError, subprocess.CalledProcessError) as error:
        print(f'Repository check could not run: {error}', file=sys.stderr)
        return 2
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    mode = 'index' if arguments.staged else 'working tree'
    print(f'Repository checks passed ({mode}, {len(files)} files).')
    return 0


if __name__ == '__main__':
    sys.exit(main())
