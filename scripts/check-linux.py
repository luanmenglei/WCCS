"""Run native Linux verification for the M2 server and shared crates."""

import os
import platform
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    if platform.system() != "Linux":
        raise SystemExit("需要实际 Linux 环境。Windows 上的交叉类型检查另行记录。")
    target = "x86_64-unknown-linux-gnu"
    commands = [
        ["rustc", "--version", "--verbose"],
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "build", "--locked", "-p", "wccs-server", "--target", target, "--release"],
        ["cargo", "clippy", "--locked", "-p", "wccs-server", "-p", "wccs-protocol", "--target", target, "--all-targets", "--", "-D", "warnings"],
        ["cargo", "test", "--locked", "-p", "wccs-server", "-p", "wccs-protocol", "--target", target],
        ["cargo", "test", "--locked", "-p", "wccs-inventory", "--lib", "--target", target],
    ]
    if os.environ.get("WCCS_TEST_DATABASE_URL"):
        # The capacity case has its own sampling window and runs separately.
        commands.append(["cargo", "test", "--locked", "-p", "wccs-server", "--target", target, "--test", "http", "--", "--ignored", "--skip", "concurrent_control_preview_commit_and_reconnect"])
    for command in commands:
        subprocess.run(command, cwd=ROOT, check=True)
    if not os.environ.get("WCCS_TEST_DATABASE_URL"):
        print("PostgreSQL 集成测试未执行。设置 WCCS_TEST_DATABASE_URL 后重跑。")


if __name__ == "__main__":
    main()
