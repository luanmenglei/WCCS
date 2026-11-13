"""Build the version 1 UTF-8 script manifest consumed by desktop maintenance."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import stat


def build(source, package_id, version, configuration_version, runtime_tool_id,
          runtime_file_version, provenance, license_name):
    for value in (package_id, version, configuration_version, runtime_tool_id):
        if not re.fullmatch(r"[A-Za-z0-9_-]{1,80}", value):
            raise ValueError("IDs must use 1-80 ASCII letters, digits, _ or -")
    files = []
    names = set()
    total = 0
    reserved = {"CON", "PRN", "AUX", "NUL"} | {
        f"{prefix}{n}" for prefix in ("COM", "LPT") for n in range(1, 10)
    }
    source = Path(source).resolve(strict=True)
    for path in sorted(source.rglob("*")):
        if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise ValueError("Reparse points are not package inputs")
        if path.is_dir():
            continue
        relative = path.relative_to(source).as_posix()
        if (len(relative) > 200 or relative.lower() == "manifest.json"
                or relative.lower() in names
                or any(not re.fullmatch(r"[A-Za-z0-9_.-]+", part)
                       or part.endswith(".")
                       or part.split(".")[0].upper() in reserved
                       for part in relative.split("/"))):
            raise ValueError(f"Unsupported Windows package path: {relative}")
        raw = path.read_bytes()
        total += len(raw)
        if total > 128 * 1024 or len(files) >= 64:
            raise ValueError("Package exceeds 64 files or 128 KiB")
        files.append({"path": relative, "content": raw.decode("utf-8"),
                      "sha256": hashlib.sha256(raw).hexdigest()})
        names.add(relative.lower())
    if not files:
        raise ValueError("Package is empty")
    for value, limit in ((runtime_file_version, 100), (provenance, 300), (license_name, 300)):
        if not value or len(value.encode("utf-8")) > limit:
            raise ValueError("Runtime version, source and license are required and bounded")
    return {"schema_version": 1, "id": package_id, "version": version,
            "configuration_version": configuration_version, "runtime_tool_id": runtime_tool_id,
            "runtime_file_version": runtime_file_version, "source": provenance,
            "license": license_name, "files": files}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source-directory", "id", "version", "configuration-version",
                 "runtime-tool-id", "runtime-file-version", "source", "license", "output"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    manifest = build(args.source_directory, args.id, args.version, args.configuration_version,
                     args.runtime_tool_id, args.runtime_file_version, args.source, args.license)
    # Exclusive creation keeps a published version from being silently overwritten.
    with Path(args.output).open("x", encoding="utf-8", newline="\n") as target:
        json.dump(manifest, target, ensure_ascii=False, indent=2)
        target.write("\n")


if __name__ == "__main__":
    main()
