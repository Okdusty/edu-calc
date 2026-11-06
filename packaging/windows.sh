#!/usr/bin/env bash
# build/qcalcp-$VERSION-win64.zip from target/release/EduCalc.exe, in an MSYS2 UCRT64 shell (see
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
cp target/release/EduCalc.exe "$D/bin/"
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
mods=("$D/bin/EduCalc.exe" "$D"/lib/gdk-pixbuf-2.0/*/loaders/*.dll "$D/lib/$PY"/lib-dynload/*.pyd)
ntldd -R "${mods[@]}" | tr '\\' '/' | grep -io "[a-z]:/[^ ]*/${P#/}/bin/[^ ]*\.dll" | sort -u | while read -r dll; do
    cp -n "$(cygpath -u "$dll")" "$D/bin/"
done

(cd build && zip -qr "$NAME.zip" "$NAME")
ls -l "build/$NAME.zip"

# for the installer (qcalcp.wxs). Windows Installer versions are numbers: 0.1.0-beta.3 -> 0.1.0.3
sed -nE '0,/^version = /s/^version = "([0-9]+\.[0-9]+\.[0-9]+)(-[a-z]+\.([0-9]+))?"/\1.\3/p' Cargo.toml | sed 's/\.$/.0/' > build/msi-version
grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' build/msi-version || { echo "can't make an installer version from Cargo.toml" >&2; exit 1; }
# its license page takes RTF
{
    printf '{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Consolas;}}\\f0\\fs16\n'
    tr -d '\f\r' < LICENSE | sed -e 's/\\/\\\\/g' -e 's/{/\\{/g' -e 's/}/\\}/g' -e 's/$/\\par/'
    printf '}\n'
} > build/LICENSE.rtf
