#!/usr/bin/env bash
# Builds the protonsync SDK helper into one executable (ADR 0025).
#
#   helper/build.sh [bun-target]      e.g. bun-linux-x64, bun-linux-x64-musl
#
# Steps: fetch the pinned SDK checkout into helper/vendor/sdk, install its CLI
# dependencies, copy our sources next to Proton's CLI code and compile with
# Proton's own build script. Result: helper/dist/protonsync-sdk-helper.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=sdk-version.env
source "$here/sdk-version.env"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/../Cargo.toml" | head -n1)"
target="${1:-bun}"
vendor="$here/vendor/sdk"
PRIVATE_NPM_MIRROR="https://pkg.protontech.ch/npm/npm-public/"
PUBLIC_NPM_REGISTRY="https://registry.npmjs.org/"
# Fail instead of hanging if the registry does not answer.
INSTALL_TIMEOUT=15m

if [ ! -d "$vendor/.git" ]; then
    git init -q "$vendor"
    git -C "$vendor" remote add origin "$SDK_REPO"
fi
if [ "$(git -C "$vendor" rev-parse HEAD 2>/dev/null || true)" != "$SDK_COMMIT" ]; then
    GIT_TERMINAL_PROMPT=0 git -C "$vendor" fetch -q --depth 1 origin "$SDK_COMMIT"
    git -C "$vendor" checkout -q --force "$SDK_COMMIT"
fi

cd "$vendor/cli"
# Proton's lockfile points every package at Proton's internal npm mirror
# (pkg.protontech.ch), which is not reachable from outside: `bun install`
# then waits forever. The mirror serves the same tarballs as the public
# registry (the sha512 integrity values in the lockfile match), so we point
# the lockfile at registry.npmjs.org; bun still checks every integrity value.
sed -i "s#$PRIVATE_NPM_MIRROR#$PUBLIC_NPM_REGISTRY#g" bun.lock
timeout "$INSTALL_TIMEOUT" bun install --frozen-lockfile
rm -rf src/protonsync
mkdir -p src/protonsync
cp "$here"/src/*.ts src/protonsync/

# Proton's build script names the app `${CLI_APP_VERSION_NAME}@${CLI_VERSION}`;
# that becomes our honest x-pm-appversion, e.g. external-drive-protonsync@0.1.0-alpha.
CLI_APP_VERSION_NAME=external-drive-protonsync \
CLI_VERSION="${version}-alpha" \
JS_VERSION="$SDK_JS_VERSION" \
CLI_OUTPUT_DIR="$here/dist" \
    bun ./scripts/build-cli.mjs src/protonsync/main.ts "$target"

mv "$here/dist/main" "$here/dist/protonsync-sdk-helper"
echo "built $here/dist/protonsync-sdk-helper"
