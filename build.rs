fn main() {
    #[cfg(windows)]
    winres::WindowsResource::new()
        .set_icon("assets/BYOHAPTICS.ico")
        .compile()
        .expect("embed application icon");
}
