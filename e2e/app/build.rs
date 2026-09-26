fn main() {
    // The harness links Donut as a library, and a dependency's link arguments
    // never reach this binary. Without the Common Controls v6 manifest Windows
    // loads comctl32 v5, and the app dies at startup with
    // STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) before WebDriver can attach.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let manifest = manifest_dir
            .parent()
            .and_then(|e2e| e2e.parent())
            .expect("e2e/app sits two levels below the repository root")
            .join("src-tauri")
            .join("app.manifest");
        // A plain backslash path: mt.exe rejects the \\?\ form canonicalize gives.
        let manifest = manifest.to_str().unwrap().replace('/', "\\");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{manifest}");
        println!("cargo:rerun-if-changed={manifest}");
    }
}
