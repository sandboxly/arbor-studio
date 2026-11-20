// Windows: puts the app icon into arbor_studio.exe, so Explorer and the taskbar show it.
// macOS: builds an app description (name, id, version) into the program, so the menu bar shows
// "ArborStudio" even when the program runs on its own, outside "Arbor Studio.app".
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "macos" {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>ArborStudio</string>
    <key>CFBundleDisplayName</key>
    <string>ArborStudio</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.arbor-studio</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#
        );
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("Info.plist");
        std::fs::write(&out, plist).expect("write Info.plist");
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", out.display());
    }

    #[cfg(windows)]
    if target_os == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "ArborStudio");
        res.set("FileDescription", "ArborStudio");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
