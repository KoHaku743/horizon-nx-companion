//! On Windows: the icon and the name the Explorer shows for the .exe.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/icon.ico")
            .set("ProductName", "Horizon NX Companion")
            .set("FileDescription", "Horizon NX Companion");
        res.compile().expect("the Windows resources");
    }
    println!("cargo:rerun-if-changed=../../packaging/windows/icon.ico");
}
