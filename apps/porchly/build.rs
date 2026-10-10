//! Gives porchly.exe its icon and name on Windows (Explorer, the taskbar).

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/brand/porchly.ico");
        res.set("ProductName", "Porchly");
        res.set("FileDescription", "Porchly");
        if let Err(e) = res.compile() {
            // A missing icon must never stop the build.
            println!("cargo:warning=couldn't add the Windows icon: {e}");
        }
    }
}
