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
  LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
  UV_INSTALL_ROOT="${ANKI_LAUNCHER_VENV_ROOT:-$HOME/Library/Application Support/AnkiProgramFiles}"
  rm -rf "$TARGET_APP"
  ditto "$APP_LAUNCHER" "$TARGET_APP"

  SRC_LAUNCHER="$APP_LAUNCHER/Contents/MacOS/launcher"
  DST_LAUNCHER="$TARGET_APP/Contents/MacOS/launcher"
  if ! cmp -s "$SRC_LAUNCHER" "$DST_LAUNCHER"; then
    echo "error: installed launcher bundle does not match built bundle at $TARGET_APP" >&2
    exit 1
  fi

  for wheel_path in "$WHEELS_DIR"/*.whl; do
    wheel_name="$(basename "$wheel_path")"
    if ! cmp -s "$wheel_path" "$TARGET_APP/Contents/Resources/wheels/$wheel_name"; then
      echo "error: installed wheel does not match built wheel: $wheel_name" >&2
      exit 1
    fi
  done

  "$LSREGISTER" -f "$TARGET_APP" >/dev/null 2>&1 || true
  rm -rf "$UV_INSTALL_ROOT/.venv"
  rm -f "$UV_INSTALL_ROOT/.sync_complete"
  rm -f "$UV_INSTALL_ROOT/uv.lock"
  echo "Installed local fork Anki to: $TARGET_APP"
else
  echo "Built local fork app bundle at: $APP_LAUNCHER"
fi
