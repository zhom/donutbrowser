fn main() {
  println!("cargo::rustc-check-cfg=cfg(mobile)");

  // Ensure dist folder exists for tauri::generate_context!() macro
  // This allows running cargo test without building the frontend first
  ensure_dist_folder_exists();

  #[cfg(target_os = "macos")]
  {
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    println!("cargo:rustc-link-lib=framework=CoreServices");
  }

  // Inject build version based on environment variables set by CI
  if let Ok(build_tag) = std::env::var("BUILD_TAG") {
    // Custom BUILD_TAG takes highest priority (used for nightly builds)
    println!("cargo:rustc-env=BUILD_VERSION={build_tag}");
  } else if let Ok(tag_name) = std::env::var("GITHUB_REF_NAME") {
    // This is set by GitHub Actions to the tag name (e.g., "v1.0.0")
    println!("cargo:rustc-env=BUILD_VERSION={tag_name}");
  } else if std::env::var("STABLE_RELEASE").is_ok() {
    // Fallback for stable releases - use CARGO_PKG_VERSION with 'v' prefix
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string());
    println!("cargo:rustc-env=BUILD_VERSION=v{version}");
  } else if let Ok(commit_hash) = std::env::var("GITHUB_SHA") {
    // For nightly builds, use timestamp format or fallback to commit hash
    let short_hash = &commit_hash[0..7.min(commit_hash.len())];
    println!("cargo:rustc-env=BUILD_VERSION=nightly-{short_hash}");
  } else {
    // Development build fallback
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".to_string());
    println!("cargo:rustc-env=BUILD_VERSION=dev-{version}");
  }

  // Inject vault password at build time
  if let Ok(vault_password) = std::env::var("DONUT_BROWSER_VAULT_PASSWORD") {
    println!("cargo:rustc-env=DONUT_BROWSER_VAULT_PASSWORD={vault_password}");
  } else {
    // Use default password if environment variable is not set
    println!("cargo:rustc-env=DONUT_BROWSER_VAULT_PASSWORD=donutbrowser-api-vault-password");
  }

  // Tell Cargo to rebuild if the proxy binary source changes
  println!("cargo:rerun-if-changed=src/bin/proxy_server.rs");
  println!("cargo:rerun-if-changed=src/proxy_server.rs");
  println!("cargo:rerun-if-changed=src/proxy_runner.rs");
  println!("cargo:rerun-if-changed=src/proxy_storage.rs");

  // Tell Cargo to rebuild when binaries directory contents change
  // This ensures tauri_build is re-run after sidecar binaries are copied
  println!("cargo:rerun-if-changed=binaries");

  // Only run tauri_build if all external binaries exist
  // This allows building donut-proxy sidecar without the other binaries present
  if external_binaries_exist() {
    tauri_build::build()
  } else {
    println!("cargo:warning=Skipping tauri_build: external binaries not found. This is expected when building sidecar binaries.");
  }
}

fn external_binaries_exist() -> bool {
  use std::env;
  use std::path::PathBuf;

  let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
    Ok(dir) => dir,
    Err(_) => return false,
  };

  let target = match env::var("TARGET") {
    Ok(t) => t,
    Err(_) => return false,
  };

  let binaries_dir = PathBuf::from(&manifest_dir).join("binaries");

  // Check for required external binaries
  let donut_proxy_name = if target.contains("windows") {
    format!("donut-proxy-{}.exe", target)
  } else {
    format!("donut-proxy-{}", target)
  };

  binaries_dir.join(&donut_proxy_name).exists()
}

fn ensure_dist_folder_exists() {
  use std::fs;
  use std::path::PathBuf;

  let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
  let dist_dir = PathBuf::from(&manifest_dir).join("..").join("dist");

  if !dist_dir.exists() {
    fs::create_dir_all(&dist_dir).expect("Failed to create dist directory");
    let index_path = dist_dir.join("index.html");
    fs::write(
      &index_path,
      "<!DOCTYPE html><html><head></head><body></body></html>",
    )
    .expect("Failed to create stub index.html");
    println!(
      "cargo:warning=Created stub dist folder for compilation. Run 'pnpm build' for full frontend."
    );
  }

  println!("cargo:rerun-if-changed=../dist");
}
