fn main() {
    #[cfg(windows)]
    winres::WindowsResource::new()
        .set_icon("assets/BYOHAPTICS.ico")
        .set("ProductName", "BYO Haptics Joy-Con Bridge")
        .set("FileDescription", "BYO Haptics Joy-Con Bridge")
        .compile()
        .expect("embed application resources");
}
