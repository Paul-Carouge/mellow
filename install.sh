#!/bin/sh
# Installs the Mellow desktop app for the current user, from the latest signed release:
#
#   curl -fsSL https://raw.githubusercontent.com/Paul-Carouge/mellow/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/Paul-Carouge/mellow/main/install.sh | sh -s -- --uninstall
#
# Linux: ~/.local/bin/mellow-desktop and a menu entry. macOS: ~/Applications/Mellow.app.
# The app then updates itself from User settings → About (ADR-0017). Nothing needs root.
set -eu

readonly REPOSITORY="Paul-Carouge/mellow"
# The release key (release/minisign.pub, ADR-0016).
readonly RELEASE_PUBLIC_KEY="RWSgQjtbJHdhFhXos863nm52YDKk2y4M35fp8Gw1zkvTrCdP7mqA6b1F"
readonly VERSION_PATTERN='^[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*$'

fail() { echo "mellow install: $*" >&2; exit 1; }
say() { echo "mellow install: $*"; }

target_triple() {
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) echo x86_64-unknown-linux-gnu ;;
    Linux-aarch64 | Linux-arm64) echo aarch64-unknown-linux-gnu ;;
    Darwin-arm64) echo aarch64-apple-darwin ;;
    *) fail "no Mellow build for $(uname -s) $(uname -m) yet" ;;
  esac
}

install_paths() {
  case "$(uname -s)" in
    Linux)
      APP_DIRECTORY="${HOME}/.local/bin"
      EXECUTABLE="${APP_DIRECTORY}/mellow-desktop"
      MENU_ENTRY="${XDG_DATA_HOME:-${HOME}/.local/share}/applications/mellow.desktop"
      ;;
    Darwin)
      APP_BUNDLE="${HOME}/Applications/Mellow.app"
      APP_DIRECTORY="${APP_BUNDLE}/Contents/MacOS"
      EXECUTABLE="${APP_DIRECTORY}/mellow-desktop"
      ;;
  esac
}

latest_version() {
  curl -fsSL --proto '=https' "https://api.github.com/repos/${REPOSITORY}/releases/latest" \
    | sed -n 's/^ *"tag_name": *"v\([^"]*\)".*$/\1/p' | head -n 1
}

# Accepts the file only if its minisign signature names it; without minisign, falls back to the
# signed digest list's line for it, which proves integrity but not authenticity.
verify() {
  asset="$1" directory="$2" base="$3"
  if command -v minisign > /dev/null 2>&1; then
    curl -fsSL --proto '=https' -o "${directory}/${asset}.minisig" "${base}/${asset}.minisig"
    minisign -V -P "${RELEASE_PUBLIC_KEY}" -m "${directory}/${asset}" -x "${directory}/${asset}.minisig" \
      | grep -Fqx "Trusted comment: ${asset}" || fail "the signature of ${asset} does not verify"
    say "signature verified"
    return
  fi
  say "minisign is not installed: checking the SHA-256 digest only (install minisign to check the signature)"
  sums="mellow-${version}-SHA256SUMS"
  curl -fsSL --proto '=https' -o "${directory}/${sums}" "${base}/${sums}"
  expected="$(awk -v name="${asset}" '$2 == name { print $1 }' "${directory}/${sums}")"
  [ -n "${expected}" ] || fail "${asset} is missing from ${sums}"
  if command -v sha256sum > /dev/null 2>&1; then
    actual="$(sha256sum "${directory}/${asset}" | awk '{ print $1 }')"
  else
    actual="$(shasum -a 256 "${directory}/${asset}" | awk '{ print $1 }')"
  fi
  [ "${actual}" = "${expected}" ] || fail "the digest of ${asset} does not match"
}

write_menu_entry() {
  mkdir -p "$(dirname "${MENU_ENTRY}")"
  cat > "${MENU_ENTRY}" << EOF
[Desktop Entry]
Type=Application
Name=Mellow
Comment=Self-hosted chat
Exec=${EXECUTABLE}
Terminal=false
Categories=Network;Chat;InstantMessaging;
EOF
}

write_app_bundle_info() {
  cat > "${APP_BUNDLE}/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Mellow</string>
  <key>CFBundleDisplayName</key><string>Mellow</string>
  <key>CFBundleIdentifier</key><string>app.mellow.desktop</string>
  <key>CFBundleExecutable</key><string>mellow-desktop</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
}

uninstall() {
  case "$(uname -s)" in
    Linux) rm -f "${EXECUTABLE}" "${EXECUTABLE}.previous" "${EXECUTABLE}.rolled-back" "${MENU_ENTRY}" ;;
    Darwin) rm -rf "${APP_BUNDLE}" ;;
  esac
  say "Mellow is uninstalled. Your sign-in stays in the system keychain until you remove it."
}

main() {
  command -v curl > /dev/null 2>&1 || fail "curl is required"
  target="$(target_triple)"
  install_paths
  if [ "${1:-}" = "--uninstall" ]; then
    uninstall
    return
  fi
  version="$(latest_version)"
  echo "${version}" | grep -Eq "${VERSION_PATTERN}" || fail "cannot tell the latest version (got '${version}')"
  asset="mellow-desktop-${version}-${target}"
  base="https://github.com/${REPOSITORY}/releases/download/v${version}"
  directory="$(mktemp -d)"
  trap 'rm -rf "${directory}"' EXIT

  say "downloading Mellow ${version} for ${target}"
  curl -fSL --proto '=https' --progress-bar -o "${directory}/${asset}" "${base}/${asset}"
  verify "${asset}" "${directory}" "${base}"

  # The app's updater refuses folders other accounts can write to, so keep them private.
  mkdir -p "${APP_DIRECTORY}"
  chmod go-w "${APP_DIRECTORY}"
  chmod 0755 "${directory}/${asset}"
  mv -f "${directory}/${asset}" "${EXECUTABLE}"
  case "$(uname -s)" in
    Linux)
      write_menu_entry
      say "installed ${EXECUTABLE} and a menu entry"
      case ":${PATH}:" in
        *":${APP_DIRECTORY}:"*) ;;
        *) say "add ${APP_DIRECTORY} to your PATH to start it as mellow-desktop from a terminal" ;;
      esac
      ;;
    Darwin)
      write_app_bundle_info
      say "installed ${APP_BUNDLE}"
      ;;
  esac
  say "Mellow ${version} is ready; later versions install from User settings → About"
}

main "$@"
