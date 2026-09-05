fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../assets/icon.ico");

        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../../assets/icon.ico")
            .set("ProductName", "Girder")
            .set("FileDescription", "Girder")
            .set("OriginalFilename", "girder.exe")
            .set("LegalCopyright", "Copyright 2026 Cory Maynard");
        resource
            .compile()
            .expect("failed to embed the Girder Windows icon");
    }
}
