"""Combine a verified client archive, public configuration and one explicit mother target."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("client_package", ROOT / "scripts/package-client.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


def read_verified_client(client):
    """验证内层包的文件清单，保留已发布的字节供镜像再次封装。"""
    with zipfile.ZipFile(client) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("duplicate client archive entry")
        files = {name: archive.read(name) for name in names}
    manifest = json.loads(files["manifest.json"])
    if manifest.get("schema_version") != 1 or manifest.get("state_schema") != 1 or manifest.get("target") != "x86_64-pc-windows-msvc":
        raise ValueError("unsupported client package")
    required_files = {
        "bin/wccs-agent.exe", "bin/wccs-session.exe", "bin/wccs-inventory.exe",
        "bin/wccs-login-provider.dll", "runtime/pwsh.exe",
    } | {"scripts/" + name for name in package.SCRIPTS}
    if not required_files.issubset(files):
        raise ValueError("incomplete client package")
    expected = {item["path"] for item in manifest["files"]} | {"manifest.json"}
    if set(files) != expected:
        raise ValueError("unlisted client material")
    for item in manifest["files"]:
        data = files[item["path"]]
        if len(data) != item["size"] or package.digest(data) != item["sha256"]:
            raise ValueError("client checksum mismatch")
    return manifest["version"], files


def image_configuration(target, certificate, version):
    """母机绑定信息只进入现场目标文件，随镜像分发的配置只含公共字段。"""
    required = {"machine_name", "machine_guid", "install_directory", "service_name", "server", "ca_sha256"}
    if set(target) != required:
        raise ValueError("mother target must contain only public target fields")
    if package.digest(certificate).lower() != target["ca_sha256"].lower():
        raise ValueError("CA checksum mismatch")
    configuration = {key: target[key] for key in ("server", "ca_sha256", "install_directory", "service_name")}
    configuration.update(schema_version=1, image_version=version)
    return configuration


def build(client, ca, target, output):
    version, files = read_verified_client(client)
    certificate = Path(ca).read_bytes()
    configuration = image_configuration(target, certificate, version)
    payload = {"image/package/" + name: data for name, data in files.items()}
    payload["image/image.json"] = (json.dumps(configuration, indent=2) + "\n").encode()
    payload["image/ca.crt"] = certificate
    payload["mother-target.json"] = (json.dumps(target, indent=2) + "\n").encode()
    payload["image-mother.ps1"] = (ROOT / "scripts/image-mother.ps1").read_bytes()
    payload["READ-ME.md"] = (ROOT / "docs/tasks/image-deployment-field-guide.md").read_bytes()
    payload["M3-M5-FIELD-GUIDE.md"] = (ROOT / "docs/tasks/m3-m5-field-guide.md").read_bytes()
    contest_spec = importlib.util.spec_from_file_location("contest_package", ROOT / "scripts/package-contest-scripts.py")
    contest_package = importlib.util.module_from_spec(contest_spec)
    contest_spec.loader.exec_module(contest_package)
    contest_manifest = contest_package.build(ROOT / "scripts/contest", "wccs-contest", "v1", "v1",
                                             "powershell", "7.6.9.500", "WCCS", "Project-internal")
    payload["contest-scripts-v1.json"] = (json.dumps(contest_manifest, ensure_ascii=False, indent=2) + "\n").encode()
    for filename, action in (("01-upgrade-mother", "Upgrade"), ("02-seal-mother", "Seal"), ("03-diagnose", "Diagnose"), ("04-recover-teaching", "Recover")):
        payload[filename + ".cmd"] = (
            '@echo off\r\n"%~dp0image\\package\\runtime\\pwsh.exe" -NoProfile -ExecutionPolicy Bypass '
            f'-File "%~dp0image-mother.ps1" -Action {action}\r\n'
            'if errorlevel 1 pause\r\nexit /b %errorlevel%\r\n'
        ).encode()
    package.write_archive(Path(output), payload, {"schema_version": 1, "version": version, "purpose": "image-preparation"})
    checksum = hashlib.sha256(Path(output).read_bytes()).hexdigest()
    Path(str(output) + ".sha256").write_text(checksum + "  " + Path(output).name + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--ca", type=Path, required=True)
    parser.add_argument("--mother-target", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build(args.client, args.ca, json.loads(args.mother_target.read_text(encoding="utf-8-sig")), args.output)


if __name__ == "__main__":
    main()
