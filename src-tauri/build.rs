fn main() {
  #[cfg(target_os = "macos")]
  {
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    println!("cargo:rustc-link-lib=framework=CoreServices");
  }

  tauri_build::build()
}
