"""Build a deterministic offline archive from explicitly built client/runtime inputs."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import struct
import subprocess
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ["client-field.ps1", "client-common.ps1", "client-system.ps1", "waiting-service.ps1", "login-test.ps1",
           "image-common.ps1", "image-deployment.ps1"]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pe_imports(data):
    """Read the native x64 PE import table, without executing build outputs."""
    if data[:2] != b"MZ":
        raise ValueError("native PE file required")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0" or struct.unpack_from("<H", data, pe + 4)[0] != 0x8664:
        raise ValueError("x64 PE file required")
    sections = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    if struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise ValueError("PE32+ required")
    table = optional + optional_size

    def offset(rva):
        for index in range(sections):
            entry = table + index * 40
            virtual_size, address, raw_size, raw_pointer = struct.unpack_from("<IIII", data, entry + 8)
            if address <= rva < address + max(virtual_size, raw_size):
                return raw_pointer + rva - address
        raise ValueError("PE RVA outside sections")

    import_rva = struct.unpack_from("<I", data, optional + 120)[0]
    if not import_rva:
        return []
    descriptor = offset(import_rva)
    imports = []
    while any(data[descriptor:descriptor + 20]):
        name = offset(struct.unpack_from("<I", data, descriptor + 12)[0])
        end = data.index(b"\0", name)
        imports.append(data[name:end].decode("ascii").lower())
        descriptor += 20
    return sorted(imports)


def dependency_notices(files):
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--offline", "--locked", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc"], cwd=ROOT
    ))
    packages = {item["id"]: item for item in metadata["packages"]}
    nodes = {item["id"]: item["dependencies"] for item in metadata["resolve"]["nodes"]}
    pending = [item["id"] for item in metadata["packages"] if item["name"] in
               ("wccs-agent", "wccs-session", "wccs-inventory")]
    included = set()
    while pending:
        identity = pending.pop()
        if identity in included:
            continue
        included.add(identity)
        pending.extend(nodes[identity])
    notices = []
    for identity in sorted(included):
        item = packages[identity]
        if item["source"] is None:
            continue
        notices.append({key: item[key] for key in ("name", "version", "license", "repository")})
        directory = Path(item["manifest_path"]).parent
        for path in directory.rglob("*"):
            if path.is_file() and re.match(r"^(LICEN[CS]E|COPYING)([._-]|$)", path.name, re.I):
                files[f"licenses/{item['name']}-{item['version']}/{path.relative_to(directory).as_posix()}"] = path.read_bytes()
    files["build/dependencies.json"] = (json.dumps(sorted(notices, key=lambda item: (item["name"], item["version"])), indent=2) + "\n").encode()


def safe_files(root):
    if root.is_symlink() or getattr(root.lstat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
        raise ValueError("reparse input directory")
    for path in sorted(root.rglob("*")):
        if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise ValueError("reparse input entry")
        if path.is_file():
            yield path


def write_archive(output, files, metadata):
    """Same input bytes and metadata yield the same archive, regardless of timestamps."""
    seen = set()
    for name in files:
        if not re.fullmatch(r"[A-Za-z0-9_./+ -]+", name) or name.startswith("/") or any(
            part in ("", ".", "..") for part in name.split("/")
        ) or name.casefold() in seen or name == "manifest.json":
            raise ValueError("unsafe or duplicate archive path")
        seen.add(name.casefold())
        for part in name.split("/"):
            if part.endswith((".", " ")) or re.match(r"^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\.|$)", part, re.I):
                raise ValueError("Windows path alias")
    manifest = metadata | {"files": [
        {"path": name, "size": len(data), "sha256": digest(data)}
        for name, data in sorted(files.items())
    ]}
    manifest_bytes = (json.dumps(manifest, ensure_ascii=False, indent=2) + "\n").encode()
    entries = files | {"manifest.json": manifest_bytes}
    with output.open("xb") as stream:
        with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for name, data in sorted(entries.items()):
                info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.create_system = 3
                info.external_attr = (stat.S_IFREG | 0o644) << 16
                archive.writestr(info, data, compresslevel=9)
    return manifest_bytes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--provider", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--runtime-version", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", args.version) or args.version.endswith("."):
        parser.error("invalid package version")
    if args.runtime_version != "7.6.9":
        parser.error("this package recipe pins PowerShell 7.6.9 x64")
    files = {}
    for name in ("wccs-agent.exe", "wccs-session.exe", "wccs-inventory.exe"):
        files["bin/" + name] = (args.binaries / name).read_bytes()
    files["bin/wccs-login-provider.dll"] = args.provider.read_bytes()
    imports = {name: pe_imports(data) for name, data in files.items()}
    for dependencies in imports.values():
        if any(name.startswith(("vcruntime", "msvcp", "msvcr")) for name in dependencies):
            raise ValueError("client must be built with the static CRT recipe")
    for path in safe_files(args.runtime):
        files["runtime/" + path.relative_to(args.runtime).as_posix()] = path.read_bytes()
    for name in ("pwsh.exe", "LICENSE.txt", "ThirdPartyNotices.txt"):
        if "runtime/" + name not in files:
            raise ValueError("runtime or license missing")
    for name in SCRIPTS:
        files["scripts/" + name] = (ROOT / "scripts" / name).read_bytes()
    files["FIELD-GUIDE.md"] = (ROOT / "docs/tasks/offline-client-field-guide.md").read_bytes()
    files["client.cmd"] = b'@echo off\r\n"%~dp0runtime\\pwsh.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\\client-field.ps1" %*\r\nexit /b %errorlevel%\r\n'
    files["target.example.json"] = (json.dumps({
        "machine_name": "LAB-PC-001", "machine_guid": "00000000-0000-0000-0000-000000000000",
        "install_directory": "C:\\Program Files\\WCCS", "service_name": "WCCS-Agent",
        "server": "https://192.0.2.10", "ca_sha256": "REPLACE_WITH_VERIFIED_CA_SHA256",
    }, indent=2) + "\n").encode()
    sources = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard"], cwd=ROOT, text=True).splitlines()
    source_hashes = {name: digest((ROOT / name).read_bytes()) for name in sorted(set(sources))
                     if (ROOT / name).is_file() and Path(name).suffix in
                     (".rs", ".toml", ".lock", ".cpp", ".h", ".def", ".ps1", ".py", ".manifest")}
    files["build/source-inputs.json"] = (json.dumps(source_hashes, sort_keys=True, indent=2) + "\n").encode()
    files["build/Cargo.lock"] = (ROOT / "Cargo.lock").read_bytes()
    files["build/rust-toolchain.toml"] = (ROOT / "rust-toolchain.toml").read_bytes()
    dependency_notices(files)
    versions = {name: tomllib.loads((ROOT / "crates" / name / "Cargo.toml").read_text())["package"]["version"]
                for name in ("wccs-agent", "wccs-session", "wccs-inventory")}
    metadata = {
        "schema_version": 1, "state_schema": 1, "version": args.version, "target": "x86_64-pc-windows-msvc",
        "windows_minimum_build": 15063, "powershell": args.runtime_version, "components": versions,
        "toolchain": {"rust": "1.99.1", "msvc": "14.44.35207", "sdk": "10.0.19041.0", "crt": "static"},
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source_files_sha256": digest(json.dumps(source_hashes, sort_keys=True).encode()),
        "cargo_lock_sha256": digest((ROOT / "Cargo.lock").read_bytes()),
        "native_imports": imports,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    manifest = write_archive(args.output, files, metadata)
    args.output.with_suffix(".manifest.json").write_bytes(manifest)
    checksum = digest(args.output.read_bytes())
    args.output.with_suffix(".sha256").write_text(checksum + "  " + args.output.name + "\n", encoding="ascii")
    print(json.dumps({"package": str(args.output), "sha256": checksum, "files": len(files), "bytes": args.output.stat().st_size}))


if __name__ == "__main__":
    main()
