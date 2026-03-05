#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJ_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
APP_LAUNCHER="$PROJ_ROOT/out/launcher/Anki.app"
RESOURCES_DIR="$APP_LAUNCHER/Contents/Resources"
WHEELS_DIR="$RESOURCES_DIR/wheels"

cd "$PROJ_ROOT"
./ninja wheels

cd "$SCRIPT_DIR"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJ_ROOT/target}"
NODMG=1 ./build.sh

mkdir -p "$WHEELS_DIR"
rm -f "$WHEELS_DIR"/*.whl
cp "$PROJ_ROOT"/out/wheels/*.whl "$WHEELS_DIR"/

VERSION="$(tr -d '[:space:]' < "$PROJ_ROOT/.version")"
sed "s/ANKI_VERSION/$VERSION/g" "$SCRIPT_DIR/pyproject.local.toml" > "$RESOURCES_DIR/pyproject.toml"
touch "$RESOURCES_DIR/local-install-mode"

if [[ "${INSTALL_TO_APPLICATIONS:-1}" == "1" ]]; then
  TARGET_APP="${ANKI_LOCAL_INSTALL_PATH:-/Applications/Anki.app}"
  rm -rf "$TARGET_APP"
  cp -R "$APP_LAUNCHER" "$TARGET_APP"
  echo "Installed local fork Anki to: $TARGET_APP"
else
  echo "Built local fork app bundle at: $APP_LAUNCHER"
fi
