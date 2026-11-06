#!/usr/bin/env bash
# The calculator engine for the browser: GMP, MPFR, libxml2 and libqalculate, then src/qalc/shim.cpp,
# compiled to WebAssembly with Emscripten (emcc on PATH). Libraries are kept in $WORK/prefix between
# runs, so a shim change only relinks. Output: web/app/engine/qalc.{mjs,wasm}
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${WORK:-$ROOT/web/engine/.build}
OUT=$ROOT/web/app/engine
P=$WORK/prefix
GMP=6.3.0 MPFR=4.2.2 XML=2.13.8 QALC=5.12.0
JOBS=$(nproc)
# cross-compiling: configure must not try to run what emcc makes
CROSS="--build=x86_64-pc-linux-gnu --host=wasm32-unknown-emscripten"
mkdir -p "$WORK/src" "$P" "$OUT"
export EM_PKG_CONFIG_PATH=$P/lib/pkgconfig PKG_CONFIG_PATH=$P/lib/pkgconfig PKG_CONFIG_LIBDIR=$P/lib/pkgconfig

# file, sha256, urls: the first server that answers, checked against the sum whichever it was
fetch() {
    local file=$WORK/src/$1 sum=$2 url
    shift 2
    for url in "$@"; do
        [ -s "$file" ] && break
        curl -fsSL --connect-timeout 20 --max-time 300 --retry 2 --retry-delay 5 -o "$file" "$url" || { echo "no answer from $url"; rm -f "$file"; }
    done
    echo "$sum  $file" | sha256sum -c -
}

if [ ! -f "$P/lib/libgmp.a" ]; then
    fetch gmp.tar.xz a3c2b80201b89e68616f4ad30bc66aee4927c3ce50e33929ca819d5c43538898 \
        "https://mirrors.kernel.org/gnu/gmp/gmp-$GMP.tar.xz" "https://ftpmirror.gnu.org/gmp/gmp-$GMP.tar.xz" "https://gmplib.org/download/gmp/gmp-$GMP.tar.xz" "https://ftp.gnu.org/gnu/gmp/gmp-$GMP.tar.xz"
    rm -rf "$WORK/gmp" && mkdir "$WORK/gmp" && tar xf "$WORK/src/gmp.tar.xz" -C "$WORK/gmp" --strip-components=1
    (cd "$WORK/gmp"
     CC_FOR_BUILD=gcc CFLAGS=-O2 emconfigure ./configure --host=none --prefix="$P" --disable-assembly --disable-shared --enable-static --disable-cxx
     emmake make -j"$JOBS" && emmake make install)
fi

if [ ! -f "$P/lib/libmpfr.a" ]; then
    fetch mpfr.tar.xz b67ba0383ef7e8a8563734e2e889ef5ec3c3b898a01d00fa0a6869ad81c6ce01 \
        "https://mirrors.kernel.org/gnu/mpfr/mpfr-$MPFR.tar.xz" "https://ftpmirror.gnu.org/mpfr/mpfr-$MPFR.tar.xz" "https://www.mpfr.org/mpfr-$MPFR/mpfr-$MPFR.tar.xz" "https://ftp.gnu.org/gnu/mpfr/mpfr-$MPFR.tar.xz"
    rm -rf "$WORK/mpfr" && mkdir "$WORK/mpfr" && tar xf "$WORK/src/mpfr.tar.xz" -C "$WORK/mpfr" --strip-components=1
    (cd "$WORK/mpfr"
     CFLAGS=-O2 emconfigure ./configure $CROSS --prefix="$P" --with-gmp="$P" --disable-shared --enable-static --disable-thread-safe
     emmake make -j"$JOBS" && emmake make install)
fi

if [ ! -f "$P/lib/libxml2.a" ]; then
    fetch xml.tar.xz 277294cb33119ab71b2bc81f2f445e9bc9435b893ad15bb2cd2b0e859a0ee84a \
        "https://download.gnome.org/sources/libxml2/${XML%.*}/libxml2-$XML.tar.xz" "https://mirror.umd.edu/gnome/sources/libxml2/${XML%.*}/libxml2-$XML.tar.xz"
    rm -rf "$WORK/xml" && mkdir "$WORK/xml" && tar xf "$WORK/src/xml.tar.xz" -C "$WORK/xml" --strip-components=1
    (cd "$WORK/xml"
     CFLAGS=-O2 emconfigure ./configure $CROSS --prefix="$P" --disable-shared --enable-static --without-python --without-zlib --without-lzma \
         --without-iconv --without-icu --without-threads --without-http --without-ftp --without-modules --without-catalog --without-debug
     emmake make -j"$JOBS" libxml2.la && emmake make install-libLTLIBRARIES install-pkgconfigDATA && emmake make -C include install)
fi

Q=$WORK/qalc
if [ ! -f "$P/lib/libqalculate.a" ]; then
    fetch qalc.tar.gz f3dfdcf97d5a9e3a5bc0ebe66afd184721b606276eee3ba0ada8c92a5e71d44c \
        "https://github.com/Qalculate/libqalculate/releases/download/v$QALC/libqalculate-$QALC.tar.gz"
    rm -rf "$Q" && mkdir "$Q" && tar xf "$WORK/src/qalc.tar.gz" -C "$Q" --strip-components=1
    # ac_cv_prog_cc_c23: configure also looks for a host compiler and reuses emcc's -std=gnu23 on it
    (cd "$Q"
     ac_cv_prog_cc_c23=no CPPFLAGS="-I$P/include" LDFLAGS="-L$P/lib" CFLAGS=-O2 CXXFLAGS="-O2 -fwasm-exceptions" \
         emconfigure ./configure $CROSS --prefix="$P" --disable-shared --enable-static --enable-compiled-definitions --disable-nls \
         --disable-textport --disable-tests --disable-insecure --without-libcurl --without-icu --without-gnuplot-call --without-readline
     emmake make -C data
     emmake make -j"$JOBS" -C libqalculate
     cp libqalculate/.libs/libqalculate.a "$P/lib/"
     mkdir -p "$P/include/libqalculate" && cp libqalculate/*.h "$P/include/libqalculate/")
fi

em++ -std=c++17 -O2 -fwasm-exceptions -I"$P/include" "$ROOT/src/qalc/shim.cpp" "$P/lib/libqalculate.a" -L"$P/lib" -lxml2 -lmpfr -lgmp \
    -o "$OUT/qalc.mjs" \
    -sMODULARIZE=1 -sEXPORT_ES6=1 -sEXPORT_NAME=createQalc -sENVIRONMENT=worker,node \
    -sALLOW_MEMORY_GROWTH=1 -sINITIAL_MEMORY=67108864 -sSTACK_SIZE=16777216 \
    -sEXPORTED_FUNCTIONS=_qs_calc,_qs_set_var,_qs_unset_var,_qs_sample,_qs_free,_qs_options,_qs_catalog,_malloc,_free \
    -sEXPORTED_RUNTIME_METHODS=ccall,UTF8ToString,HEAPF64
ls -l "$OUT"
