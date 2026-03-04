#!/usr/bin/env nu
# release.nu — build, sign, notarize, and package bop for distribution
#
# Requirements:
#   - Developer ID Application cert in keychain
#   - NOTARIZE_APPLE_ID and NOTARIZE_PASSWORD (app-specific) in env
#     OR stored in keychain profile "bop-notary" (xcrun notarytool store-credentials)

# Extract version from Cargo.toml content string
def extract_version [cargo_content: string]: nothing -> string {
  $cargo_content
    | lines
    | where ($it | str starts-with "version")
    | first
    | parse --regex 'version\s*=\s*"([^"]+)"'
    | get capture0.0
}

def run_tests [] {
  use std/assert

  # Test version extraction from typical Cargo.toml content
  let cargo = "[package]\nname = \"bop\"\nversion = \"0.3.1\"\nedition = \"2021\""
  assert equal (extract_version $cargo) "0.3.1"

  # Test with extra spacing
  let cargo2 = "version  =  \"1.2.3\""
  assert equal (extract_version $cargo2) "1.2.3"

  # Test version with pre-release
  let cargo3 = "version = \"0.1.0-alpha.1\""
  assert equal (extract_version $cargo3) "0.1.0-alpha.1"

  # Test with temp file round-trip
  let tmp = (mktemp)
  "[package]\nname = \"test\"\nversion = \"2.5.0\"\n" | save --force $tmp
  let from_file = (extract_version (open --raw $tmp))
  assert equal $from_file "2.5.0"
  rm $tmp

  print "PASS: release.nu"
}

def main [
  --version: string = ""  # Version string (default: read from Cargo.toml)
  --test                   # Run internal self-tests
] {
  if $test {
    run_tests
    return
  }
  let root = ($env.FILE_PWD | path dirname)

  mut ver = $version
  if ($ver | is-empty) {
    $ver = (extract_version (open --raw $"($root)/crates/jc/Cargo.toml"))
  }

  let dist = $"($root)/dist/bop-($ver)-macos"
  mkdir $dist

  print $"-- bop release ($ver) --"

  # -- 1. Detect signing identity --
  let sign_raw = (^security find-identity -v -p codesigning
    | lines
    | where ($it | str contains "Developer ID Application")
    | first)

  if ($sign_raw | is-empty) {
    print -e "No 'Developer ID Application' cert found in keychain."
    print -e "  Install your cert, then re-run this script."
    exit 1
  }

  let sign_id = ($sign_raw | parse --regex '"([^"]+)"' | get capture0.0)
  print $"  signing identity: ($sign_id)"

  let team_id = ($sign_id | parse --regex '\(([A-Z0-9]+)\)' | get capture0.0)
  print $"  team ID: ($team_id)"

  # -- 2. Build CLI (Release) --
  print "\n-- CLI --"
  ^cargo build --release --manifest-path $"($root)/Cargo.toml"
  cp $"($root)/target/release/bop" $"($dist)/bop"
  ^codesign --force --sign $sign_id --options runtime $"($dist)/bop"
  print $"  bop signed: ($dist)/bop"

  # -- 3. Build QL app (Release) --
  print "\n-- QL extension --"
  ^xcodebuild -project $"($root)/macos/macos.xcodeproj" -scheme JobCardHost -configuration Release $"DEVELOPMENT_TEAM=($team_id)" $"CODE_SIGN_IDENTITY=($sign_id)" CODE_SIGN_STYLE=Manual build

  let derived = (^xcodebuild -project $"($root)/macos/macos.xcodeproj" -scheme JobCardHost -configuration Release -showBuildSettings
    | lines
    | where ($it | str contains "BUILT_PRODUCTS_DIR")
    | first
    | str trim
    | split column " = "
    | get column2.0
    | str trim)

  let app = $"($derived)/JobCardHost.app"
  cp -r $app $"($dist)/JobCardHost.app"
  print "  JobCardHost.app built"

  # -- 4. Notarize --
  print "\n-- notarize --"
  ^ditto -c -k --keepParent $"($dist)/JobCardHost.app" $"($dist)/JobCardHost.zip"

  if ("NOTARIZE_APPLE_ID" in $env) {
    ^xcrun notarytool submit $"($dist)/JobCardHost.zip" --apple-id $env.NOTARIZE_APPLE_ID --password $env.NOTARIZE_PASSWORD --team-id $team_id --wait
  } else {
    ^xcrun notarytool submit $"($dist)/JobCardHost.zip" --keychain-profile "bop-notary" --wait
  }

  ^xcrun stapler staple $"($dist)/JobCardHost.app"
  print "  notarized and stapled"

  # -- 5. Package --
  print "\n-- package --"
  cd $"($root)/dist"
  ^tar czf $"bop-($ver)-macos-arm64.tar.gz" $"bop-($ver)-macos/"
  print $"  ($root)/dist/bop-($ver)-macos-arm64.tar.gz"

  let sha = (^shasum -a 256 $"bop-($ver)-macos-arm64.tar.gz" | split column " " | get column1.0)
  print $"  sha256: ($sha)"
  $"($sha)  bop-($ver)-macos-arm64.tar.gz" | save $"bop-($ver)-macos-arm64.tar.gz.sha256"

  print "\n-- done --"
  print $"  CLI:    ($dist)/bop"
  print $"  QL app: ($dist)/JobCardHost.app"
  print $"  tar.gz: ($root)/dist/bop-($ver)-macos-arm64.tar.gz"
  print $"  sha256: ($sha)"
  print ""
  print "Next: upload tar.gz + sha256 to GitHub release"
  print "      brew formula: url + sha256 -> Formula/bop.rb"
}
