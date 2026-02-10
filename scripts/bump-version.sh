#!/usr/bin/env bash
set -euo pipefail

NEW_VERSION=${1:-}
DRY_RUN=false
COMMIT=false
TAG=false

shift_args=()
while (("$#")); do
  case "$1" in
    --dry-run) DRY_RUN=true ; shift ;;
    --commit) COMMIT=true ; shift ;;
    --tag) TAG=true ; shift ;;
    *) shift_args+=("$1") ; shift ;;
  esac
done

if [[ -z "${NEW_VERSION}" ]]; then
    echo "Usage: $0 <version> [--dry-run] [--commit] [--tag]"
    exit 1
fi
NEW_VERSION=${NEW_VERSION#v}

# Find Cargo.toml files tracked by git to avoid touching unrelated files
FILES=$(git ls-files -- "**/Cargo.toml" Cargo.toml || true)
if [[ -z "$FILES" ]]; then
    echo "No Cargo.toml files found in repo." >&2
    exit 1
fi

changed_files=()

# Use awk to safely update package.version and internal path deps' version lines
for f in $FILES; do
  tmp=$(mktemp)
  awk -v ver="$NEW_VERSION" '
    BEGIN { in_pkg = 0 }
    /^\s*\[package\]\s*$/ { in_pkg = 1; print; next }
    /^\s*\[.*\]/ { in_pkg = 0; print; next }
    {
      if (in_pkg && $0 ~ /^\s*version\s*=/) {
        sub(/version\s*=\s*"[^"]+"/, "version = \"" ver "\"")
      }
      print
    }
  ' "$f" > "$tmp"

  if ! cmp -s "$f" "$tmp"; then
    changed_files+=("$f")
    if $DRY_RUN; then
      echo "=== $f ==="
      diff -u "$f" "$tmp" || true
    else
      mv "$tmp" "$f"
      echo "Updated $f"
    fi
  else
    rm -f "$tmp"
  fi
done

if [[ ${#changed_files[@]} -eq 0 ]]; then
  echo "No changes necessary."
  exit 0
fi

# Run checks before committing
run_checks() {
  echo "Running cargo fmt (check)..."
  cargo fmt --all -- --check
  echo "Running clippy..."
  cargo clippy --all -- -D warnings
  echo "Running tests..."
  cargo test --all --workspace
  echo "Running cargo audit..."
  cargo audit
}

if ! $DRY_RUN; then
  if ! run_checks; then
    echo "Checks failed; aborting. Revert or fix and re-run." >&2
    exit 1
  fi
  git add "${changed_files[@]}"
  git commit -m "chore(release): v${NEW_VERSION}"
  if $TAG; then
    git tag "v${NEW_VERSION}"
    git push origin HEAD
    git push origin "v${NEW_VERSION}"
  fi
  echo "Bump complete and committed."
else
  echo "Dry-run completed; no files were changed."
fi

