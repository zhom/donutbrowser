fn main() {
  println!("cargo::rustc-check-cfg=cfg(mobile)");

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

  // Check for both required external binaries
  let nodecar_name = if target.contains("windows") {
    format!("nodecar-{}.exe", target)
  } else {
    format!("nodecar-{}", target)
  };

  let donut_proxy_name = if target.contains("windows") {
    format!("donut-proxy-{}.exe", target)
  } else {
    format!("donut-proxy-{}", target)
  };

  let nodecar_exists = binaries_dir.join(&nodecar_name).exists();
  let donut_proxy_exists = binaries_dir.join(&donut_proxy_name).exists();

  nodecar_exists && donut_proxy_exists
}
