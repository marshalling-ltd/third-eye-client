#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT_DIR

NEXT_VERSION="$(
python3 <<'PY'
import os
import re
import sys
from pathlib import Path

root = Path(os.environ["ROOT_DIR"])
cargo_toml_path = root / "Cargo.toml"
cargo_lock_path = root / "Cargo.lock"

toml_text = cargo_toml_path.read_text(encoding="utf-8")
version_match = re.search(r'^version = "(\d+)\.(\d+)\.(\d+)"$', toml_text, re.MULTILINE)
if not version_match:
    sys.exit("Could not find package version in Cargo.toml.")

major = int(version_match.group(1))
minor = int(version_match.group(2))
patch = int(version_match.group(3))
next_version = f"{major}.{minor}.{patch + 1}"

toml_text = (
    toml_text[: version_match.start()]
    + f'version = "{next_version}"'
    + toml_text[version_match.end() :]
)
cargo_toml_path.write_text(toml_text, encoding="utf-8")

lock_text = cargo_lock_path.read_text(encoding="utf-8")
lock_pattern = r'(\[\[package\]\]\nname = "third-eye-client"\nversion = ")(\d+\.\d+\.\d+)(")'

def replace_lock_version(match: re.Match[str]) -> str:
    return f'{match.group(1)}{next_version}{match.group(3)}'

lock_text, replacements = re.subn(lock_pattern, replace_lock_version, lock_text, count=1)
if replacements != 1:
    sys.exit("Could not update third-eye-client package version in Cargo.lock.")

cargo_lock_path.write_text(lock_text, encoding="utf-8")
print(next_version)
PY
)"

printf 'Bumped project version to %s\n' "$NEXT_VERSION"
