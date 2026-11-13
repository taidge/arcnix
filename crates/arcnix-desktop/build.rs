fn main() {
    println!("cargo:rerun-if-changed=assets/arcnix.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/arcnix.ico")
            .set("ProductName", "Arcnix")
            .set("FileDescription", "Arcnix Server Workbench")
            .compile()
            .expect("compile Arcnix application icon resource");
    }
}
