//! Builds the C++ shim over libqalculate (src/qalc/shim.cpp); flags come from pkg-config.
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/qalc/shim.cpp");
    let out = Command::new("pkg-config").args(["--cflags", "--libs", "libqalculate"]).output().expect("pkg-config not found");
    assert!(out.status.success(), "pkg-config: libqalculate not found: {}", String::from_utf8_lossy(&out.stderr));
    let flags = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut b = cc::Build::new();
    b.cpp(true).std("c++17").file("src/qalc/shim.cpp");
    let mut libs = vec![];
    for f in flags.split_whitespace() {
        if let Some(d) = f.strip_prefix("-I") {
            b.include(d);
        } else if let Some(l) = f.strip_prefix("-l") {
            libs.push(l.to_string());
        } else if let Some(d) = f.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={d}");
        } else {
            b.flag(f);
        }
    }
    b.compile("qalcshim");
    // after the static shim so the linker resolves its symbols against these
    for l in libs {
        println!("cargo:rustc-link-lib={l}");
    }
}
