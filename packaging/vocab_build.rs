//! The companion's horizon-mod-vocab: the API horizon-nx's build generated (generated/, kept by
//! packaging/vendor-horizon.sh), written where the crate's `include!`s look for it, with the
//! crate's own folder in place of the `@HORIZON_MOD_VOCAB@` the vendoring left in its `#[path]`s.
fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let dir = manifest.join("generated");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // Forward slashes: the path goes into string literals, where a Windows backslash would escape.
    let crate_dir = manifest.to_string_lossy().replace('\\', "/");
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in std::fs::read_dir(&dir).expect("generated/") {
        let path = entry.expect("a generated file").path();
        let text = std::fs::read_to_string(&path).expect("reading a generated file");
        std::fs::write(
            out.join(path.file_name().unwrap()),
            text.replace("@HORIZON_MOD_VOCAB@", &crate_dir),
        )
        .expect("writing a generated file");
    }
}
