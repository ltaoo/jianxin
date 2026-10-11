//! Embed the shared application icon into both Windows executables.
use std::{env, fs, path::PathBuf};

pub fn main() {
    println!("cargo:rerun-if-changed=../../platform/windows/build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest
        .join("../../assets/jianxin.ico")
        .canonicalize()
        .expect("missing Windows app icon: assets/jianxin.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    let resource = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("jianxin.rc");
    // RC accepts forward slashes; avoid escapes and the Windows extended-path prefix.
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    let icon_path = icon_path.strip_prefix("//?/").unwrap_or(&icon_path);
    fs::write(&resource, format!("1 ICON \"{icon_path}\"\n"))
        .expect("cannot write Windows icon resource");
    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_required()
        .expect("cannot embed the Jianxin Windows app icon");
}
