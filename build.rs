//! Tells the installer binary where the compiled `p9ime.dll` will be so it can
//! `include_bytes!` it.
//!
//! Build order is guaranteed by Cargo: the installer bin depends on this
//! package's lib, and the lib's rlib and cdylib (`p9ime.dll`) are produced by
//! the same rustc invocation and uplifted into the artifact directory before
//! any dependent target compiles. rustc records the included file in dep-info,
//! so the installer is rebuilt whenever the DLL changes.
//!
//! `cargo check` / `clippy` never link the cdylib; use `--no-default-features`
//! for those (see the `embed-dll` feature in Cargo.toml).

use std::env;
use std::path::PathBuf;

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
}
