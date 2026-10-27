//! Embeds the icon and the version information into the Windows executables.
//!
//! `slydecode.exe` and `slyde.exe` get the SlydeCode mark and a description, so
//! Explorer, the taskbar and Task Manager show a product instead of a bare file
//! name. Other targets skip this entirely. A machine without a resource compiler
//! (`rc.exe` or `windres`) still builds: the executables just keep the default
//! icon, and a build warning says so.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use embed_resource::CompilationResult;

const ICON: &str = "../../assets/brand/slydecode.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let icon = manifest_dir.join(ICON);
    if !icon.is_file() {
        println!("cargo:warning=icon not found at {}", icon.display());
        return;
    }
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());

    let rc = out_dir.join("slyde.rc");
    if let Err(err) = fs::write(&rc, resource_script(&icon, &version)) {
        println!("cargo:warning=could not write {}: {err}", rc.display());
        return;
    }
    // `compile_for` emits `rustc-link-arg-bin`, which Cargo rejects for a package
    // that declares `rust-version`; the generic form is accepted. Both binaries of
    // this package want the same resource anyway.
    // Neither outcome fails the build: an icon is not worth blocking a release.
    match embed_resource::compile_for_everything(&rc, embed_resource::NONE) {
        CompilationResult::Ok | CompilationResult::NotWindows => {}
        CompilationResult::NotAttempted(why) => println!(
            "cargo:warning=no resource compiler found, so the executables keep the default icon: {why}"
        ),
        CompilationResult::Failed(why) => {
            println!("cargo:warning=could not embed the icon: {why}");
        }
    }
}

/// The `.rc` source: the icon plus the version block Windows displays.
fn resource_script(icon: &Path, version: &str) -> String {
    let mut parts = version
        .split(|ch: char| !ch.is_ascii_digit())
        .map(|part| part.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    // The resource compiler wants forward slashes; backslashes are escapes.
    let icon = icon.display().to_string().replace('\\', "/");
    format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "ProductName", "SlydeCode"
      VALUE "FileDescription", "SlydeCode"
      VALUE "ProductVersion", "{version}"
      VALUE "FileVersion", "{version}"
      VALUE "LegalCopyright", "MIT License"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    )
}
