#!/bin/bash
NEW_VERSION=${1}
if [[ -z "${NEW_VERSION}" ]]; then
    echo "Usage: ${0} <version> [--dry-run] [--commit] [--tag]"
    exit 1
fi

# Remove 'v' prefix if present
NEW_VERSION=${NEW_VERSION#v}

# Prefer the safe Python bump script
if [[ -x "$(pwd)/scripts/bump_version_safe.py" ]]; then
    echo "Using scripts/bump_version_safe.py to update versions (safe mode)"
    python3 scripts/bump_version_safe.py "${NEW_VERSION}" "$@"
    exit $?
fi

# Fallback (legacy) behavior
echo "Safe bump script not available. Falling back to legacy sed-based bump (less safe)."

echo "Bumping versions to ${NEW_VERSION}..."

# Update package versions
find . -name "Cargo.toml" -exec sed -i "s/^version = \".*\"/version = \"${NEW_VERSION}\"/" {} +

# Update internal dependency versions
find . -name "Cargo.toml" -exec sed -i "s/version = \".*\", path = \"\.\.\//version = \"${NEW_VERSION}\", path = \"\.\.\//" {} +

echo "Done."
