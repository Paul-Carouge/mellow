#!/bin/sh
# Installs the Mellow desktop app for the current user, without root (ADR-0055):
#   ~/.local/bin/mellow, its desktop entry, icons and AppStream metainfo under
#   ${XDG_DATA_HOME:-~/.local/share}. `./install.sh --uninstall` removes them again.
set -eu

package_directory=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
bin_directory="${HOME}/.local/bin"
data_directory="${XDG_DATA_HOME:-${HOME}/.local/share}"
icon_sizes="16 24 32 48 64 128 256 512"

installed_files() {
  echo "${bin_directory}/mellow"
  echo "${data_directory}/applications/mellow.desktop"
  echo "${data_directory}/metainfo/mellow.metainfo.xml"
  echo "${data_directory}/icons/hicolor/scalable/apps/mellow.svg"
  for size in ${icon_sizes}; do
    echo "${data_directory}/icons/hicolor/${size}x${size}/apps/mellow.png"
  done
}

refresh_desktop_caches() {
  # Optional: launchers that watch these directories pick the changes up without them.
  if command -v update-desktop-database > /dev/null 2>&1; then
    update-desktop-database -q "${data_directory}/applications" \
      || echo "warning: update-desktop-database failed; the launcher may need a new session" >&2
  fi
  if command -v gtk-update-icon-cache > /dev/null 2>&1; then
    gtk-update-icon-cache -q -t "${data_directory}/icons/hicolor" \
      || echo "warning: gtk-update-icon-cache failed; the icon may need a new session" >&2
  fi
}

case "${1:-}" in
  --uninstall)
    installed_files | while IFS= read -r file; do rm -f -- "${file}"; done
    refresh_desktop_caches
    echo "Mellow is removed. Your settings stay in ~/.config/mellow and your session in the system's secret store."
    exit 0
    ;;
  "") ;;
  *)
    echo "usage: $0 [--uninstall]" >&2
    exit 2
    ;;
esac

# The app links the system's FFmpeg, PipeWire, ALSA and GPU libraries: say which are missing
# before installing something that cannot start.
if command -v ldd > /dev/null 2>&1; then
  missing=$(ldd "${package_directory}/bin/mellow" | grep 'not found' || true)
  if [ -n "${missing}" ]; then
    echo "Mellow cannot start: these libraries are missing:" >&2
    echo "${missing}" >&2
    echo "Install them with your distribution's packages (README: \"Desktop app on Linux\")." >&2
    echo "A missing libavcodec.so.N means this package was built for another FFmpeg version:" >&2
    echo "download the Mellow package named after the FFmpeg your distribution ships (ffmpeg -version)." >&2
    exit 1
  fi
fi

mkdir -p "${bin_directory}"
# Copied beside, then renamed: a running Mellow keeps its old file instead of crashing.
cp -- "${package_directory}/bin/mellow" "${bin_directory}/.mellow.new"
chmod 755 "${bin_directory}/.mellow.new"
mv -f -- "${bin_directory}/.mellow.new" "${bin_directory}/mellow"

copy_file() {
  mkdir -p "$(dirname -- "$2")"
  cp -- "$1" "$2"
  chmod 644 "$2"
}

# Launchers often start apps without ~/.local/bin in PATH: name the binary by its full path when
# the path needs no quoting in a desktop entry, and otherwise rely on PATH.
desktop_entry="${data_directory}/applications/mellow.desktop"
copy_file "${package_directory}/share/applications/mellow.desktop" "${desktop_entry}"
case "${bin_directory}" in
  *[!A-Za-z0-9._/-]*)
    echo "warning: ${bin_directory} must be in PATH for the launcher to start Mellow" >&2
    ;;
  *)
    sed "s|^Exec=mellow\$|Exec=${bin_directory}/mellow|" "${desktop_entry}" > "${desktop_entry}.new"
    mv -f -- "${desktop_entry}.new" "${desktop_entry}"
    ;;
esac
copy_file "${package_directory}/share/metainfo/mellow.metainfo.xml" \
  "${data_directory}/metainfo/mellow.metainfo.xml"
copy_file "${package_directory}/share/icons/hicolor/scalable/apps/mellow.svg" \
  "${data_directory}/icons/hicolor/scalable/apps/mellow.svg"
for size in ${icon_sizes}; do
  copy_file "${package_directory}/share/icons/hicolor/${size}x${size}/apps/mellow.png" \
    "${data_directory}/icons/hicolor/${size}x${size}/apps/mellow.png"
done
refresh_desktop_caches

echo "Mellow is installed: ${bin_directory}/mellow, and in your applications menu."
