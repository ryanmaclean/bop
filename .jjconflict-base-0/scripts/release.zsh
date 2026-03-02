#!/usr/bin/env zsh
# release.zsh — build, sign, notarize, and package bop for distribution
#
# Usage:
#   ./scripts/release.zsh [--version 0.6.0]
#
# Requirements:
#   - Developer ID Application cert in keychain
#   - NOTARIZE_APPLE_ID and NOTARIZE_PASSWORD (app-specific) in env
#     OR stored in keychain profile "bop-notary" (xcrun notarytool store-credentials)
#
set -euo pipefail
ROOT=${0:A:h:h}

VERSION=${1:-}
if [[ -z "$VERSION" ]]; then
    # Pull from Cargo.toml
    VERSION=$(grep '^version' "${ROOT}/crates/jc/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
fi

DIST="${ROOT}/dist/bop-${VERSION}-macos"
mkdir -p "${DIST}"

echo "── bop release ${VERSION} ──"

# ── 1. Detect signing identity ────────────────────────────────────────────────

SIGN_ID=$(security find-identity -v -p codesigning 2>/dev/null \
    | grep "Developer ID Application" | head -1 \
    | sed 's/.*"\(.*\)".*/\1/')

if [[ -z "${SIGN_ID}" ]]; then
    echo "✗ No 'Developer ID Application' cert found in keychain." >&2
    echo "  Install your cert, then re-run this script." >&2
    exit 1
fi
echo "  signing identity: ${SIGN_ID}"

TEAM_ID=$(echo "${SIGN_ID}" | grep -oE '\([A-Z0-9]+\)' | tr -d '()')
echo "  team ID: ${TEAM_ID}"

# ── 2. Build CLI (Release) ────────────────────────────────────────────────────

echo "\n── CLI ──"
cargo build --release --manifest-path "${ROOT}/Cargo.toml"
cp "${ROOT}/target/release/bop" "${DIST}/bop"
codesign --force --sign "${SIGN_ID}" --options runtime "${DIST}/bop"
echo "✓ bop signed: ${DIST}/bop"

# ── 3. Build QL app (Release) ─────────────────────────────────────────────────

echo "\n── QL extension ──"
xcodebuild \
    -project "${ROOT}/macos/macos.xcodeproj" \
    -scheme JobCardHost \
    -configuration Release \
    DEVELOPMENT_TEAM="${TEAM_ID}" \
    CODE_SIGN_IDENTITY="${SIGN_ID}" \
    CODE_SIGN_STYLE=Manual \
    build 2>&1 | grep -E "error:|BUILD SUCCEEDED|BUILD FAILED"

DERIVED=$(xcodebuild -project "${ROOT}/macos/macos.xcodeproj" \
    -scheme JobCardHost -configuration Release \
    -showBuildSettings 2>/dev/null | grep "BUILT_PRODUCTS_DIR" | head -1 | awk '{print $3}')

APP="${DERIVED}/JobCardHost.app"
cp -R "${APP}" "${DIST}/JobCardHost.app"
echo "✓ JobCardHost.app built"

# ── 4. Notarize ───────────────────────────────────────────────────────────────

echo "\n── notarize ──"

# Zip for notarization
ditto -c -k --keepParent "${DIST}/JobCardHost.app" "${DIST}/JobCardHost.zip"

if [[ -n "${NOTARIZE_APPLE_ID:-}" ]]; then
    xcrun notarytool submit "${DIST}/JobCardHost.zip" \
        --apple-id "${NOTARIZE_APPLE_ID}" \
        --password "${NOTARIZE_PASSWORD}" \
        --team-id "${TEAM_ID}" \
        --wait
else
    xcrun notarytool submit "${DIST}/JobCardHost.zip" \
        --keychain-profile "bop-notary" \
        --wait
fi

xcrun stapler staple "${DIST}/JobCardHost.app"
echo "✓ notarized and stapled"

# ── 5. Package ────────────────────────────────────────────────────────────────

echo "\n── package ──"
cd "${ROOT}/dist"
tar czf "bop-${VERSION}-macos-arm64.tar.gz" "bop-${VERSION}-macos/"
echo "✓ ${ROOT}/dist/bop-${VERSION}-macos-arm64.tar.gz"

SHA=$(shasum -a 256 "bop-${VERSION}-macos-arm64.tar.gz" | awk '{print $1}')
echo "  sha256: ${SHA}"
echo "${SHA}  bop-${VERSION}-macos-arm64.tar.gz" > "bop-${VERSION}-macos-arm64.tar.gz.sha256"

echo "\n── done ──"
echo "  CLI:    ${DIST}/bop"
echo "  QL app: ${DIST}/JobCardHost.app"
echo "  tar.gz: ${ROOT}/dist/bop-${VERSION}-macos-arm64.tar.gz"
echo "  sha256: ${SHA}"
echo ""
echo "Next: upload tar.gz + sha256 to GitHub release"
echo "      brew formula: url + sha256 → Formula/bop.rb"
