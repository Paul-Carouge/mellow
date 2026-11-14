#!/bin/sh
# Installs the Mellow desktop app for the current user, without root (ADR-0055):
#   ~/.local/bin/mellow and its YouTube player window ~/.local/bin/mellow-player, its desktop entry, icons and AppStream metainfo under
#   ${XDG_DATA_HOME:-~/.local/share}. `./install.sh --uninstall` removes them again.
set -eu

package_directory=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
bin_directory="${HOME}/.local/bin"
data_directory="${XDG_DATA_HOME:-${HOME}/.local/share}"
icon_sizes="16 24 32 48 64 128 256 512"

installed_files() {
  echo "${bin_directory}/mellow"
  echo "${bin_directory}/mellow-player"
  echo "${data_directory}/applications/mellow.desktop"
  echo "${data_directory}/metainfo/org.mellow.desktop.metainfo.xml"
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

# The YouTube player window (ADR-0061) shows YouTube's player in WebKitGTK, which plays video
# through GStreamer. Without them Mellow still starts and opens videos in the browser instead.
player_notice=""
if command -v ldd > /dev/null 2>&1; then
  player_missing=$(ldd "${package_directory}/bin/mellow-player" | grep 'not found' || true)
  if [ -n "${player_missing}" ]; then
    # ldd's lines start with a tab: keep the library's name only.
    missing_name=$(printf '%s\n' "${player_missing}" | sed -n '1s/^[[:space:]]*\([^[:space:]]*\).*/\1/p')
    player_notice="WebKitGTK 4.1 is missing (${missing_name})"
  fi
fi
if [ -z "${player_notice}" ]; then
  # autoaudiosink, from GStreamer's good plugins: WebKitGTK's media cannot play without it.
  # gst-inspect-1.0 knows where the distribution puts plugins; the usual folders otherwise.
  gstreamer_found=""
  if command -v gst-inspect-1.0 > /dev/null 2>&1 \
    && gst-inspect-1.0 --exists autoaudiosink > /dev/null 2>&1; then
    gstreamer_found=yes
  fi
  for directory in /usr/lib/gstreamer-1.0 /usr/lib64/gstreamer-1.0 \
    /usr/lib/x86_64-linux-gnu/gstreamer-1.0 /usr/lib/aarch64-linux-gnu/gstreamer-1.0; do
    if [ -e "${directory}/libgstautodetect.so" ]; then gstreamer_found=yes; fi
  done
  if [ -z "${gstreamer_found}" ]; then
    player_notice="GStreamer's base and good plugins seem to be missing"
  fi
fi
if [ -z "${player_notice}" ]; then
  # WebKitGTK runs the player's web content in bubblewrap, with xdg-dbus-proxy in front of D-Bus
  # (ADR-0061); without them the player cannot start and videos open in the browser.
  for tool in bwrap xdg-dbus-proxy; do
    if ! command -v "${tool}" > /dev/null 2>&1; then
      player_notice="${tool} is missing (the player's sandbox)"
    fi
  done
fi
if [ -n "${player_notice}" ]; then
  echo "note: ${player_notice}: YouTube videos will open in your browser instead of Mellow's player." >&2
  echo "      Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav bubblewrap xdg-dbus-proxy" >&2
  echo "      Fedora: sudo dnf install webkit2gtk4.1 gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugin-libav bubblewrap xdg-dbus-proxy" >&2
  echo "      Arch: sudo pacman -S --needed webkit2gtk-4.1 gst-plugins-base gst-plugins-good gst-libav bubblewrap xdg-dbus-proxy" >&2
fi

mkdir -p "${bin_directory}"
# Copied beside, then renamed: a running Mellow keeps its old file instead of crashing.
for executable in mellow-player mellow; do
  cp -- "${package_directory}/bin/${executable}" "${bin_directory}/.${executable}.new"
  chmod 755 "${bin_directory}/.${executable}.new"
  mv -f -- "${bin_directory}/.${executable}.new" "${bin_directory}/${executable}"
done

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
copy_file "${package_directory}/share/metainfo/org.mellow.desktop.metainfo.xml" \
  "${data_directory}/metainfo/org.mellow.desktop.metainfo.xml"
copy_file "${package_directory}/share/icons/hicolor/scalable/apps/mellow.svg" \
  "${data_directory}/icons/hicolor/scalable/apps/mellow.svg"
for size in ${icon_sizes}; do
  copy_file "${package_directory}/share/icons/hicolor/${size}x${size}/apps/mellow.png" \
    "${data_directory}/icons/hicolor/${size}x${size}/apps/mellow.png"
done
refresh_desktop_caches

echo "Mellow is installed: ${bin_directory}/mellow, and in your applications menu."
