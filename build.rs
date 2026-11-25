fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("app.ico");
        res.set("ProductName", "Liubendou Agent");
        res.set("FileDescription", "Liubendou Agent launcher");
        res.set("OriginalFilename", "launcher.exe");
        res.set("LegalCopyright", "");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon resource skipped: {e}");
        }
    }
}