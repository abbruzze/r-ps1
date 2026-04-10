fn main() {
    #[cfg(target_os = "windows")]
    {
        match embed_resource::compile("resources/icon.rc",embed_resource::NONE).manifest_required() {
            Ok(_) => println!("cargo:warning=Icon added successfully!"),
            Err(e) => println!("cargo:warning=Error adding icon: {}", e),
        }
    }
}