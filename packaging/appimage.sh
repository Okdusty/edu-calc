#!/usr/bin/env bash
# build/qcalcp-$VERSION-x86_64.AppImage from target/release/qcalcp (run on Ubuntu 24.04, see
# .github/workflows/release.yml). GTK 4 comes in through linuxdeploy's GTK plugin; Python's
# standard library goes to usr/lib, where main.rs points PYTHONHOME at start-up. libqalculate must
# be built with --enable-compiled-definitions: its data path would point outside the AppImage.
set -euo pipefail
VERSION=${VERSION:-snapshot}
APPDIR=build/AppDir
TOOLS=build/tools
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib" "$APPDIR/usr/share/icons" "$TOOLS"
install -m755 target/release/qcalcp "$APPDIR/usr/bin/"

# Python's standard library, without tests, GUI toolkits and build files (libpython itself is a
# dependency of the binary and gets deployed with the rest)
PY=$(python3 -c 'import sys; print("python%d.%d" % sys.version_info[:2])')
cp -a "/usr/lib/$PY" "$APPDIR/usr/lib/"
(
    cd "$APPDIR/usr/lib/$PY"
    rm -rf test idlelib tkinter turtledemo ensurepip lib2to3 site-packages dist-packages config-*
    find . -name __pycache__ -prune -exec rm -rf {} +
    mkdir site-packages
)
mkdir -p "$APPDIR/apprun-hooks"
cp packaging/apprun-hook.sh "$APPDIR/apprun-hooks/zz-qcalcp.sh"
# symbolic icons for the header and plot bar buttons, whatever the host's theme is
cp -a /usr/share/icons/Adwaita /usr/share/icons/hicolor "$APPDIR/usr/share/icons/"

[ -x "$TOOLS/linuxdeploy" ] || curl -fsSL -o "$TOOLS/linuxdeploy" \
    https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
[ -x "$TOOLS/linuxdeploy-plugin-gtk" ] || curl -fsSL -o "$TOOLS/linuxdeploy-plugin-gtk" \
    https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
chmod +x "$TOOLS"/*

export PATH="$PWD/$TOOLS:$PATH" APPIMAGE_EXTRACT_AND_RUN=1 DEPLOY_GTK_VERSION=4
export LDAI_OUTPUT="build/qcalcp-$VERSION-x86_64.AppImage"
linuxdeploy --appdir "$APPDIR" \
    --desktop-file packaging/qcalcp.desktop --icon-file packaging/qcalcp.svg \
    --deploy-deps-only "$APPDIR/usr/lib/$PY/lib-dynload" \
    --plugin gtk
# GTK's GStreamer media module would load the host's GStreamer against the bundled GLib (and fail
# loudly); qcalcp plays no media
rm -rf "$APPDIR"/usr/lib/gtk-4.0/*/media
linuxdeploy --appdir "$APPDIR" --output appimage
ls -l "$LDAI_OUTPUT"
