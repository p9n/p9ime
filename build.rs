//! Build script with two jobs:
//!
//! 1. Tell the installer binary where the compiled `p9ime.dll` will be so it
//!    can `include_bytes!` it.
//!
//!    Build order is guaranteed by Cargo: the installer bin depends on this
//!    package's lib, and the lib's rlib and cdylib (`p9ime.dll`) are produced
//!    by the same rustc invocation and uplifted into the artifact directory
//!    before any dependent target compiles. rustc records the included file in
//!    dep-info, so the installer is rebuilt whenever the DLL changes.
//!
//!    `cargo check` / `clippy` never link the cdylib; use
//!    `--no-default-features` for those (see the `embed-dll` feature in
//!    Cargo.toml).
//!
//! 2. Compile `assets/icon/p9ime.rc` (the application icon) into a COFF object
//!    with `windres` and link it into every Windows artifact: the cdylib
//!    (`p9ime.dll`, whose icon index 0 is used by TSF and the uninstall key's
//!    `DisplayIcon`) and the installer `.exe` (whose first icon Explorer shows).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // OUT_DIR = <target-dir>/<triple>/<profile>/build/<pkg>-<hash>/out
    //            ancestors: 0 = out, 1 = <pkg>-<hash>, 2 = build, 3 = <profile>
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let artifact_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR has the expected cargo layout")
        .to_path_buf();

    let dll = artifact_dir.join("p9ime.dll");
    println!("cargo:rustc-env=P9IME_DLL={}", dll.display());

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        link_icon_resource(&out_dir);
    }
}

/// Compiles the icon resource script and asks rustc to link the resulting
/// object into the cdylib and all binaries of this package.
fn link_icon_resource(out_dir: &Path) {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"));
    let icon_dir = manifest_dir.join("assets").join("icon");
    let rc = icon_dir.join("p9ime.rc");
    let ico = icon_dir.join("p9ime.ico");
    println!("cargo:rerun-if-changed={}", rc.display());
    println!("cargo:rerun-if-changed={}", ico.display());
    println!("cargo:rerun-if-env-changed=WINDRES");

    // Only the GNU toolchain is supported (we cross-compile with mingw-w64).
    // Honour $WINDRES so an unusual toolchain layout can still be used.
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let windres = env::var("WINDRES").unwrap_or_else(|_| {
        if target_env == "gnu" {
            "x86_64-w64-mingw32-windres".to_string()
        } else {
            "windres".to_string()
        }
    });

    let obj = out_dir.join("p9ime_icon.o");
    let status = Command::new(&windres)
        .arg("--input")
        .arg(&rc)
        .arg("--output")
        .arg(&obj)
        .arg("--output-format=coff")
        // Resolve the .ico path referenced by the .rc relative to the .rc file.
        .arg("--include-dir")
        .arg(&icon_dir)
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "failed to run `{windres}` to compile {}: {err}",
                rc.display()
            )
        });
    assert!(
        status.success(),
        "`{windres}` failed with {status} while compiling {}",
        rc.display()
    );

    println!("cargo:rustc-link-arg={}", obj.display());
}
