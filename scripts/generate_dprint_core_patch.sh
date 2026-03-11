#!/usr/bin/env bash

set -e

# Base commit where dprint-core was vendored/referenced
# Default to the initial vendoring commit if not provided
BASE_COMMIT="${1:-${DPRINT_BASE_COMMIT:-88f6407fb9f7087362a69db5b81ef8e0baa2176f}}"
SOURCE_DIR="dprint-crates/core"
TARGET_DIR="crates/core"
PATCH_FILE="dprint-core.patch"

echo "Generating patch for $SOURCE_DIR since $BASE_COMMIT..."

# Generate the diff and use sed to replace the paths
# We use a temporary file to avoid issues with piping and potential errors
git diff "$BASE_COMMIT" -- "$SOURCE_DIR" > raw.patch

if [ ! -s raw.patch ]; then
    echo "No changes found in $SOURCE_DIR since $BASE_COMMIT."
    rm raw.patch
    exit 0
fi

# Replace 'a/dprint-crates/core/' with 'a/crates/core/' and 'b/dprint-crates/core/' with 'b/crates/core/'
sed -e "s|a/$SOURCE_DIR/|a/$TARGET_DIR/|g" \
    -e "s|b/$SOURCE_DIR/|b/$TARGET_DIR/|g" \
    raw.patch > "$PATCH_FILE"

rm raw.patch

echo "Patch generated: $PATCH_FILE"
echo "You can apply it to the original dprint repo with:"
echo "  cd /code/dprint/dprint && patch -p1 < $(realpath $PATCH_FILE)"
