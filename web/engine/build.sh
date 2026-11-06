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
mkdir -p "$WORK/src" "$P" "$OUT"
export EM_PKG_CONFIG_PATH=$P/lib/pkgconfig PKG_CONFIG_PATH=$P/lib/pkgconfig PKG_CONFIG_LIBDIR=$P/lib/pkgconfig

fetch() { # file url
    [ -s "$WORK/src/$1" ] || curl -fsSL --retry 5 --retry-delay 5 -o "$WORK/src/$1" "$2"
}

if [ ! -f "$P/lib/libgmp.a" ]; then
    fetch gmp.tar.xz "https://ftp.gnu.org/gnu/gmp/gmp-$GMP.tar.xz"
    rm -rf "$WORK/gmp" && mkdir "$WORK/gmp" && tar xf "$WORK/src/gmp.tar.xz" -C "$WORK/gmp" --strip-components=1
    (cd "$WORK/gmp"
     CC_FOR_BUILD=gcc CFLAGS=-O2 emconfigure ./configure --host=none --prefix="$P" --disable-assembly --disable-shared --enable-static --disable-cxx
     emmake make -j"$JOBS" && emmake make install)
fi

if [ ! -f "$P/lib/libmpfr.a" ]; then
    fetch mpfr.tar.xz "https://ftp.gnu.org/gnu/mpfr/mpfr-$MPFR.tar.xz"
    rm -rf "$WORK/mpfr" && mkdir "$WORK/mpfr" && tar xf "$WORK/src/mpfr.tar.xz" -C "$WORK/mpfr" --strip-components=1
    (cd "$WORK/mpfr"
     CFLAGS=-O2 emconfigure ./configure --prefix="$P" --with-gmp="$P" --disable-shared --enable-static --disable-thread-safe
     emmake make -j"$JOBS" && emmake make install)
fi

if [ ! -f "$P/lib/libxml2.a" ]; then
    fetch xml.tar.xz "https://download.gnome.org/sources/libxml2/${XML%.*}/libxml2-$XML.tar.xz"
    rm -rf "$WORK/xml" && mkdir "$WORK/xml" && tar xf "$WORK/src/xml.tar.xz" -C "$WORK/xml" --strip-components=1
    (cd "$WORK/xml"
     CFLAGS=-O2 emconfigure ./configure --prefix="$P" --disable-shared --enable-static --without-python --without-zlib --without-lzma \
         --without-iconv --without-icu --without-threads --without-http --without-ftp --without-modules --without-catalog --without-debug
     emmake make -j"$JOBS" libxml2.la && emmake make install-libLTLIBRARIES install-pkgconfigDATA && emmake make -C include install)
fi

Q=$WORK/qalc
if [ ! -f "$P/lib/libqalculate.a" ]; then
    fetch qalc.tar.gz "https://github.com/Qalculate/libqalculate/releases/download/v$QALC/libqalculate-$QALC.tar.gz"
    rm -rf "$Q" && mkdir "$Q" && tar xf "$WORK/src/qalc.tar.gz" -C "$Q" --strip-components=1
    # ac_cv_prog_cc_c23: configure also looks for a host compiler and reuses emcc's -std=gnu23 on it
    (cd "$Q"
     ac_cv_prog_cc_c23=no CPPFLAGS="-I$P/include" LDFLAGS="-L$P/lib" CFLAGS=-O2 CXXFLAGS="-O2 -fwasm-exceptions" \
         emconfigure ./configure --prefix="$P" --disable-shared --enable-static --enable-compiled-definitions --disable-nls \
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
