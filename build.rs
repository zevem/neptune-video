fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/icons/neptune-video.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/neptune-video.ico")
            .set("ProductName", "Neptune Video")
            .set("FileDescription", "Neptune Video editor")
            .compile()?;
    }
    Ok(())
}
