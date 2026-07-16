#!/usr/bin/env python3
"""
Repair legacy local media layout.

Old builds stored downloads as:
  <data_root>/media/<media_id>/<name>

New builds store files flat under:
  <data_root>/media/<name>

This script:
1) moves legacy files into the flat layout,
2) moves sidecar files (for example *_telemetry.jpg) out of legacy folders,
3) updates stale `media_sync.local_path` entries in SQLite,
4) removes empty legacy directories.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import platform
import shutil
import sqlite3
from pathlib import Path


def candidate_data_roots() -> list[Path]:
    home = Path.home()
    system = platform.system()
    if system == "Darwin":
        return [home / "Library/Application Support/eu.marshalling.third-eye-client"]
    if system == "Windows":
        appdata = os.environ.get("APPDATA")
        if not appdata:
            return []
        base = Path(appdata)
        return [
            base / "eu" / "marshalling" / "third-eye-client",
            base / "marshalling" / "third-eye-client",
            base / "eu.marshalling.third-eye-client",
        ]
    # Linux / other unix-like.
    return [
        home / ".local/share/eu.marshalling.third-eye-client",
        home / ".local/share/third-eye-client",
        home / ".local/share/marshalling/third-eye-client",
    ]


def resolve_data_root(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit).expanduser().resolve()
    for candidate in candidate_data_roots():
        if (candidate / "state.db").exists() or (candidate / "media").exists():
            return candidate
    # Fall back to first candidate to provide a predictable path in output.
    candidates = candidate_data_roots()
    if candidates:
        return candidates[0]
    raise SystemExit("Could not infer a default data root for this platform; pass --data-root.")


def safe_local_name(name: str) -> str:
    value = Path(name).name
    return value or "unnamed-media"


def file_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def same_content(left: Path, right: Path) -> bool:
    if not left.exists() or not right.exists():
        return False
    if left.stat().st_size != right.stat().st_size:
        return False
    return file_hash(left) == file_hash(right)


def collision_path(media_root: Path, filename: str, token: str) -> Path:
    original = Path(filename)
    stem = original.stem
    suffix = original.suffix
    cleaned = "".join(ch for ch in token if ch.isalnum())[:8] or "dup"
    candidate = media_root / f"{stem}__{cleaned}{suffix}"
    index = 2
    while candidate.exists():
        candidate = media_root / f"{stem}__{cleaned}_{index}{suffix}"
        index += 1
    return candidate


def move_file(src: Path, dst: Path, dry_run: bool) -> bool:
    if src == dst:
        return False
    if dry_run:
        print(f"[dry-run] move {src} -> {dst}")
        return True
    dst.parent.mkdir(parents=True, exist_ok=True)
    try:
        src.replace(dst)
    except OSError:
        shutil.move(str(src), str(dst))
    return True


def prune_empty_dirs(start: Path, media_root: Path, dry_run: bool) -> int:
    removed = 0
    current = start
    while current != media_root and current.is_relative_to(media_root):
        if not current.exists():
            current = current.parent
            continue
        try:
            if dry_run:
                # Only announce if it is actually empty.
                next(current.iterdir())
                break
            current.rmdir()
            removed += 1
        except (OSError, StopIteration):
            break
        current = current.parent
    return removed


def migrate_rows(
    conn: sqlite3.Connection,
    data_root: Path,
    media_root: Path,
    dry_run: bool,
) -> tuple[int, int, int, int]:
    rows = conn.execute(
        """
        SELECT media_id, name, local_path
          FROM media_sync
         WHERE local_path IS NOT NULL
           AND local_path <> ''
        """
    ).fetchall()

    updates: list[tuple[str, str, str]] = []
    moved_files = 0
    moved_sidecars = 0
    repaired_stale_paths = 0
    missing_files = 0

    for media_id, name, local_path in rows:
        old_path = Path(local_path).expanduser()
        if not old_path.is_absolute():
            old_path = (data_root / old_path).resolve()
        preferred = media_root / safe_local_name(name)

        if old_path.exists():
            target = preferred
            if target.exists() and old_path != target and not same_content(old_path, target):
                target = collision_path(media_root, target.name, media_id)
            if move_file(old_path, target, dry_run):
                moved_files += 1
            updates.append((str(target), media_id, name))

            # Move sidecar files out of the legacy directory too.
            parent = old_path.parent
            if parent != media_root and parent.exists() and parent.is_dir():
                for sidecar in sorted(parent.iterdir()):
                    if not sidecar.is_file():
                        continue
                    sidecar_target = media_root / sidecar.name
                    if sidecar_target.exists() and not same_content(sidecar, sidecar_target):
                        sidecar_target = collision_path(media_root, sidecar.name, media_id)
                    if move_file(sidecar, sidecar_target, dry_run):
                        moved_sidecars += 1
                prune_empty_dirs(parent, media_root, dry_run)
            continue

        # Original path is stale; keep the DB consistent if the flat path exists.
        if preferred.exists():
            updates.append((str(preferred), media_id, name))
            repaired_stale_paths += 1
        else:
            missing_files += 1

    if updates and not dry_run:
        conn.executemany(
            """
            UPDATE media_sync
               SET local_path = ?1
             WHERE media_id = ?2
               AND name = ?3
            """,
            updates,
        )

    return moved_files, moved_sidecars, repaired_stale_paths, missing_files


def flatten_remaining_dirs(media_root: Path, dry_run: bool) -> tuple[int, int]:
    moved = 0
    removed_dirs = 0
    if not media_root.exists():
        return moved, removed_dirs

    directories = sorted([path for path in media_root.iterdir() if path.is_dir()])
    for directory in directories:
        files = [p for p in directory.rglob("*") if p.is_file()]
        for file_path in files:
            target = media_root / file_path.name
            if target.exists() and not same_content(file_path, target):
                target = collision_path(media_root, file_path.name, directory.name)
            if move_file(file_path, target, dry_run):
                moved += 1
        removed_dirs += prune_empty_dirs(directory, media_root, dry_run)
    return moved, removed_dirs


def main() -> None:
    parser = argparse.ArgumentParser(description="Flatten legacy third-eye-client media layout.")
    parser.add_argument(
        "--data-root",
        help="Path containing state.db and media/ (defaults to platform-specific app data dir).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show planned actions without writing files or updating SQLite.",
    )
    args = parser.parse_args()

    data_root = resolve_data_root(args.data_root)
    db_path = data_root / "state.db"
    media_root = data_root / "media"

    print(f"Data root: {data_root}")
    print(f"DB path:   {db_path}")
    print(f"Media dir: {media_root}")
    if not db_path.exists():
        raise SystemExit(f"state.db not found at {db_path}")
    media_root.mkdir(parents=True, exist_ok=True)

    conn = sqlite3.connect(db_path)
    try:
        moved_files, moved_sidecars, repaired_stale, missing = migrate_rows(
            conn, data_root, media_root, args.dry_run
        )
        moved_extra, removed_dirs = flatten_remaining_dirs(media_root, args.dry_run)
        if args.dry_run:
            conn.rollback()
        else:
            conn.commit()
    finally:
        conn.close()

    print()
    print("Migration summary:")
    print(f"  media files moved:           {moved_files}")
    print(f"  sidecar files moved:         {moved_sidecars}")
    print(f"  stale DB paths repaired:     {repaired_stale}")
    print(f"  missing DB file references:  {missing}")
    print(f"  extra nested files moved:    {moved_extra}")
    print(f"  empty directories removed:   {removed_dirs}")
    if args.dry_run:
        print("Dry-run complete. No changes were written.")


if __name__ == "__main__":
    main()
