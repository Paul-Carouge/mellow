#!/bin/bash
# Signs Mellow.app with the release's stable self-signed certificate (ADR-0055), from a
# temporary keychain that is deleted on exit, so the identity never stays on the machine.
#
#   MACOS_SIGNING_CERTIFICATE           the PKCS#12 file (certificate and private key), base64
#   MACOS_SIGNING_CERTIFICATE_PASSWORD  its password
#   sign-app.sh <Mellow.app> <entitlements.plist>
set -euo pipefail

app="${1:?usage: sign-app.sh <Mellow.app> <entitlements.plist>}"
entitlements="${2:?usage: sign-app.sh <Mellow.app> <entitlements.plist>}"
: "${MACOS_SIGNING_CERTIFICATE:?the base64 PKCS#12 is not set}"
: "${MACOS_SIGNING_CERTIFICATE_PASSWORD:?its password is not set}"

work_directory="$(mktemp -d)"
keychain="${work_directory}/mellow-signing.keychain-db"
cleanup() {
  security delete-keychain "${keychain}" 2>/dev/null || true
  rm -rf "${work_directory}"
}
trap cleanup EXIT

umask 077
# The keychain's own password only guards it for the few seconds it exists.
keychain_password="$(openssl rand -hex 24)"
printf '%s' "${MACOS_SIGNING_CERTIFICATE}" | base64 --decode > "${work_directory}/identity.p12"
security create-keychain -p "${keychain_password}" "${keychain}"
security set-keychain-settings -lut 900 "${keychain}"
security unlock-keychain -p "${keychain_password}" "${keychain}"
security import "${work_directory}/identity.p12" -k "${keychain}" \
  -P "${MACOS_SIGNING_CERTIFICATE_PASSWORD}" -T /usr/bin/codesign
# Lets codesign use the key without a confirmation dialog.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "${keychain_password}" \
  "${keychain}" > /dev/null

# A self-signed certificate is not trusted, so `find-identity -v` hides it: name the identity by
# its SHA-1 fingerprint, which codesign accepts whatever the trust settings.
identity="$(security find-identity -p codesigning "${keychain}" \
  | sed -n 's/^ *[0-9][0-9]*) \([0-9A-F]\{40\}\) .*/\1/p' | head -n 1)"
if [[ -z "${identity}" ]]; then
  echo "the PKCS#12 holds no code-signing identity" >&2
  exit 1
fi

# --timestamp=none: Apple's timestamp service only vouches for Apple-issued certificates.
codesign --force --options runtime --timestamp=none --entitlements "${entitlements}" \
  --keychain "${keychain}" --sign "${identity}" "${app}"
codesign --verify --strict --verbose=2 "${app}"
# The designated requirement names the certificate: TCC keeps the camera, microphone and screen
# permissions across updates as long as it stays the same.
codesign --display --requirements - "${app}"
echo "signed with the certificate whose SHA-1 is ${identity}"
