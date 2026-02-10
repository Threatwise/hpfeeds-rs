#!/bin/bash
NEW_VERSION=${1}
if [[ -z "${NEW_VERSION}" ]]; then
    echo "Usage: ${0} <version>"
    exit 1
fi

# Remove 'v' prefix if present
NEW_VERSION=${NEW_VERSION#v}

echo "Bumping versions to ${NEW_VERSION}..."

# Update package versions
find . -name "Cargo.toml" -exec sed -i "s/^version = \".*\"/version = \"${NEW_VERSION}\"/" {} +

# Update internal dependency versions
find . -name "Cargo.toml" -exec sed -i "s/version = \".*\", path = \"\.\.\//version = \"${NEW_VERSION}\", path = \"\.\.\//" {} +

echo "Done."
