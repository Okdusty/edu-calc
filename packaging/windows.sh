#!/usr/bin/env bash
# build/qcalcp-$VERSION-win64.zip from target/release/qcalcp.exe, in an MSYS2 UCRT64 shell (see
# .github/workflows/release.yml). The layout mirrors the MSYS2 prefix, so GTK, gdk-pixbuf,
# libqalculate (bin\..\share\qalculate) and Python (main.rs sets PYTHONHOME to bin\..) all find
# their files relative to the exe.
set -euo pipefail
shopt -s nullglob
VERSION=${VERSION:-snapshot}
NAME=qcalcp-$VERSION-win64
D=build/$NAME
P=${MINGW_PREFIX:-/ucrt64}
rm -rf "$D" "build/$NAME.zip"
mkdir -p "$D/bin" "$D/lib" "$D/share/glib-2.0/schemas" "$D/share/icons"
cp target/release/qcalcp.exe "$D/bin/"
cp README.md LICENSE "$D/"

cp -r "$P/lib/gdk-pixbuf-2.0" "$D/lib/"
cp "$P/share/glib-2.0/schemas/gschemas.compiled" "$D/share/glib-2.0/schemas/"
cp -r "$P/share/icons/Adwaita" "$P/share/icons/hicolor" "$D/share/icons/"
[ -d "$P/share/qalculate" ] && cp -r "$P/share/qalculate" "$D/share/"

# Python's standard library, without tests, GUI toolkits and build files
PY=$(basename "$(ls -d "$P"/lib/python3.* | sort -V | tail -1)")
cp -r "$P/lib/$PY" "$D/lib/"
(
    cd "$D/lib/$PY"
    rm -rf test idlelib tkinter turtledemo ensurepip lib2to3 site-packages config-*
    find . -name __pycache__ -prune -exec rm -rf {} +
    mkdir site-packages
)

# every DLL from the prefix that the exe, the pixbuf loaders or Python's extension modules load
mods=("$D/bin/qcalcp.exe" "$D"/lib/gdk-pixbuf-2.0/*/loaders/*.dll "$D/lib/$PY"/lib-dynload/*.pyd)
ntldd -R "${mods[@]}" | tr '\\' '/' | grep -io "[a-z]:/[^ ]*/${P#/}/bin/[^ ]*\.dll" | sort -u | while read -r dll; do
    cp -n "$(cygpath -u "$dll")" "$D/bin/"
done

(cd build && zip -qr "$NAME.zip" "$NAME")
ls -l "build/$NAME.zip"
