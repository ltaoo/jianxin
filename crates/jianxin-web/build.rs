use std::{env, path::PathBuf, process::Command};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let webpc = manifest.join("../../webpc");
    for source in [
        ".design",
        "build.mjs",
        "package.json",
        "package-lock.json",
        "src",
        "packages",
        "vendor",
    ] {
        println!("cargo:rerun-if-changed={}", webpc.join(source).display());
    }
    println!("cargo:rerun-if-env-changed=NODE");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("webpc");
    let node = env::var_os("NODE").unwrap_or_else(|| "node".into());
    let status = Command::new(node)
        .arg(webpc.join("build.mjs"))
        .arg("--out-dir")
        .arg(output)
        .status()
        .expect("Node.js is required to build the Web client");
    assert!(
        status.success(),
        "Web client build failed; run npm ci --prefix webpc"
    );
}
