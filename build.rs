fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/app-icon.ico")
            .set("FileDescription", "图片反馈 · Image Feedback")
            .set("ProductName", "Image Feedback")
            .set("OriginalFilename", "ImageFeedback.exe");
        resource
            .compile()
            .expect("Could not embed the Windows application icon");
    }
    slint_build::compile("ui/app.slint").expect("Could not compile the Slint interface");
}
