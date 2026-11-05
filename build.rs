//! Embeds the icon and version details that Explorer and Task Manager show.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // The version is embedded here too, so a bump must rebuild the resources.
    println!("cargo:rerun-if-changed=Cargo.toml");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icon.ico")
        .set("ProductName", "Thermal Eye")
        .set("FileDescription", "Thermal Eye");
    resource.compile().expect("could not embed the Windows resources");
}
