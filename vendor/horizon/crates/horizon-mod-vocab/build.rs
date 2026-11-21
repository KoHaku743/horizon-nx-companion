//! The companion's horizon-mod-vocab: the API horizon-nx's build generated (generated/, kept by
//! packaging/vendor-horizon.sh), copied where the crate's `include!`s look for it.
fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let dir = manifest.join("generated");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in std::fs::read_dir(&dir).expect("generated/") {
        let path = entry.expect("a generated file").path();
        std::fs::copy(&path, out.join(path.file_name().unwrap())).expect("copying a generated file");
    }
}
