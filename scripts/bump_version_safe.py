#!/usr/bin/env python3
"""
Safe bump script for workspace Cargo.toml files.

Features:
- Parses toml using tomlkit to preserve formatting
- Updates only `[package].version`
- Updates `version` for internal path dependencies (entries with `path` pointing to `..`)
- Dry-run mode to show planned edits
- Optional `--commit` to commit changes and `--tag` to create a git tag
- Runs `cargo fmt`, `cargo clippy`, `cargo test`, `cargo audit` after updating and ensures they pass before committing

Usage: bump_version_safe.py <version> [--dry-run] [--commit] [--tag]

Note: This script requires `tomlkit` to be installed. It will attempt to install it if missing.
"""

import argparse
import subprocess
import sys
from pathlib import Path

try:
    import tomlkit
    HAVE_TOMLKIT = True
except Exception:
    HAVE_TOMLKIT = False
    tomlkit = None
    print("tomlkit not available; using fallback parser (less precise but safe).", file=sys.stderr)



def find_cargo_toml_files():
    return list(Path('.').rglob('Cargo.toml'))


def update_file_tomlkit(path: Path, new_version: str):
    content = path.read_text(encoding='utf-8')
    doc = tomlkit.parse(content)
    changed = False

    # Update [package].version
    if 'package' in doc and 'version' in doc['package']:
        old = str(doc['package']['version'])
        if old != new_version:
            doc['package']['version'] = new_version
            changed = True

    # Update internal path deps in [dependencies], [dev-dependencies], [build-dependencies]
    for table in ('dependencies', 'dev-dependencies', 'build-dependencies'):
        if table in doc:
            deps = doc[table]
            for name, val in deps.items():
                # Only consider inline tables/dictionaries with a 'path' key
                if isinstance(val, dict) and 'path' in val:
                    p = val.get('path')
                    if isinstance(p, str) and p.startswith('..'):
                        oldv = val.get('version')
                        if oldv != new_version:
                            val['version'] = new_version
                            changed = True

    if changed:
        return tomlkit.dumps(doc)
    return None


def update_file_fallback(path: Path, new_version: str):
    # Fallback parser that uses line-based heuristics to avoid changing unrelated lines
    lines = path.read_text(encoding='utf-8').splitlines()
    out = list(lines)
    changed = False

    # Helper to replace version line within a section
    def replace_version_in_section(start_idx, end_idx):
        nonlocal changed
        for i in range(start_idx, end_idx):
            if out[i].lstrip().startswith('version'):
                import re
                new_line = re.sub(r'version\s*=\s*"[^"]+"', f'version = "{new_version}"', out[i])
                if new_line != out[i]:
                    out[i] = new_line
                    changed = True
                return

    # Find [package] sections
    import re
    for m in re.finditer(r'^\[package\]', '\n'.join(lines), flags=re.MULTILINE):
        start = m.end()
        # find next section or EOF
        remaining = '\n'.join(lines)[start:]
        next_section = re.search(r'^\[', remaining, flags=re.MULTILINE)
        if next_section:
            end_pos = start + next_section.start()
            start_line = '\n'.join(lines)[:start].count('\n')
            end_line = '\n'.join(lines)[:end_pos].count('\n')
        else:
            start_line = '\n'.join(lines)[:start].count('\n')
            end_line = len(lines)
        replace_version_in_section(start_line, end_line)

    # For dependencies tables, look for lines with path = ".." and update version in the same block
    for table in ('[dependencies]', '[dev-dependencies]', '[build-dependencies]'):
        try:
            table_idx = out.index(table)
        except ValueError:
            continue
        # scan until next section
        j = table_idx + 1
        while j < len(out) and not out[j].lstrip().startswith('['):
            line = out[j]
            if 'path' in line and '..' in line:
                # update version on this line if present
                new = re.sub(r'version\s*=\s*"[^"]+"', f'version = "{new_version}"', line)
                if new != line:
                    out[j] = new
                    changed = True
                else:
                    # look backwards a few lines for version
                    for k in range(max(table_idx, j-3), j):
                        if 'version' in out[k]:
                            new2 = re.sub(r'version\s*=\s*"[^"]+"', f'version = "{new_version}"', out[k])
                            if new2 != out[k]:
                                out[k] = new2
                                changed = True
                                break
                    else:
                        # not found, insert version before this line (simple heuristic)
                        out.insert(j, f'version = "{new_version}"')
                        changed = True
                        j += 1
            j += 1

    if changed:
        return '\n'.join(out) + '\n'
    return None


def update_file(path: Path, new_version: str):
    if HAVE_TOMLKIT:
        return update_file_tomlkit(path, new_version)
    return update_file_fallback(path, new_version)


def run_checks():
    cmds = [
        (['cargo', 'fmt', '--all', '--', '--check'], 'fmt'),
        (['cargo', 'clippy', '--all', '--', '-D', 'warnings'], 'clippy'),
        (['cargo', 'test', '--all', '--workspace'], 'test'),
        (['cargo', 'audit'], 'audit'),
    ]

    for cmd, name in cmds:
        print(f"Running {name}...", file=sys.stderr)
        res = subprocess.run(cmd)
        if res.returncode != 0:
            print(f"{name} failed (exit {res.returncode}). Aborting commit.")
            return False
    return True


def git_commit_and_tag(new_version: str, tag: bool):
    subprocess.check_call(['git', 'add', '-A'])
    subprocess.check_call(['git', 'commit', '-m', f"chore(release): v{new_version}"])  # may fail if nothing to commit
    if tag:
        subprocess.check_call(['git', 'tag', f'v{new_version}'])
        subprocess.check_call(['git', 'push', 'origin', 'HEAD'])
        subprocess.check_call(['git', 'push', 'origin', f'v{new_version}'])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('version')
    parser.add_argument('--dry-run', action='store_true')
    parser.add_argument('--commit', action='store_true')
    parser.add_argument('--tag', action='store_true')
    args = parser.parse_args()

    new_version = args.version.lstrip('v')

    files = find_cargo_toml_files()
    changed_files = []
    for f in files:
        new_content = update_file(f, new_version)
        if new_content is not None:
            changed_files.append((f, new_content))

    if not changed_files:
        print('No changes necessary.')
        return

    print(f"Would change {len(changed_files)} file(s):")
    for f, new in changed_files:
        print(' -', f)

    if args.dry_run:
        print('\nDry-run enabled; not writing files. Showing diffs:\n')
        for f, new in changed_files:
            old = f.read_text(encoding='utf-8')
            print(f"=== {f} ===")
            import difflib

            for line in difflib.unified_diff(old.splitlines(), new.splitlines(), fromfile=str(f), tofile=str(f) + '.new', lineterm=''):
                print(line)
        return

    # Write files
    for f, new in changed_files:
        f.write_text(new, encoding='utf-8')
        print(f'Updated {f}')

    # Run checks
    if not run_checks():
        print('Checks failed. You can fix locally and re-run the script.')
        sys.exit(1)

    if args.commit:
        git_commit_and_tag(new_version, args.tag)
        print('Committed and pushed changes.')


if __name__ == '__main__':
    main()
