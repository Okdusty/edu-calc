//! Builds the C++ shim over libqalculate (src/qalc/shim.cpp); flags come from pkg-config.
use std::process::Command;
use std::{env, path::PathBuf};

fn compile_icons() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=packaging/qcalcp.gresource.xml");
    for size in [16, 24, 32, 48, 64, 128, 256] {
        println!("cargo:rerun-if-changed=packaging/icons/{size}x{size}/apps/qcalcp.png");
    }
    let status = Command::new("glib-compile-resources")
        .arg("packaging/qcalcp.gresource.xml").arg("--sourcedir=packaging")
        .arg("--target").arg(out.join("qcalcp.gresource"))
        .status().expect("glib-compile-resources not found (install the GTK development tools)");
    assert!(status.success(), "failed to compile the application icons");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") { return; }
    println!("cargo:rerun-if-changed=packaging/qcalcp.rc");
    println!("cargo:rerun-if-changed=packaging/qcalcp.ico");
    let resource = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let target = env::var("TARGET").unwrap();
        let tool = cc::windows_registry::find_tool(&target, "rc.exe").expect("Windows SDK resource compiler not found");
        let path = out.join("qcalcp.res");
        let status = tool.to_command().arg("/nologo").arg("/Ipackaging")
            .arg(format!("/fo{}", path.display())).arg("packaging/qcalcp.rc").status().unwrap();
        assert!(status.success(), "failed to compile the Windows icon");
        path
    } else {
        println!("cargo:rerun-if-env-changed=WINDRES");
        let path = out.join("qcalcp-icon.o");
        let status = Command::new(env::var_os("WINDRES").unwrap_or_else(|| "windres".into()))
            .args(["-I", "packaging", "-i", "packaging/qcalcp.rc", "-O", "coff", "-o"])
            .arg(&path).status().expect("windres not found (install MinGW binutils)");
        assert!(status.success(), "failed to compile the Windows icon");
        path
    };
    println!("cargo:rustc-link-arg-bin=EduCalc={}", resource.display());
}

fn main() {
    compile_icons();
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
