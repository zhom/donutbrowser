fn main() {
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

  // Ensure the proxy binary exists before Tauri checks for it
  // Tauri looks for binaries in the binaries/ directory relative to the manifest
  ensure_proxy_binary_exists();

  tauri_build::build()
}

fn ensure_proxy_binary_exists() {
  use std::env;
  use std::path::PathBuf;

  let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
    Ok(dir) => dir,
    Err(_) => return,
  };

  let target = match env::var("TARGET") {
    Ok(t) => t,
    Err(_) => return,
  };

  let binaries_dir = PathBuf::from(&manifest_dir).join("binaries");
  let binary_name = format!("donut-proxy-{}", target);
  let binary_path = binaries_dir.join(&binary_name);

  // If binary doesn't exist, try to copy it from target directory
  if !binary_path.exists() {
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let source_binary_name = if target.contains("windows") {
      "donut-proxy.exe"
    } else {
      "donut-proxy"
    };

    let source_dir = if target == env::var("HOST").unwrap_or_default() {
      format!("{manifest_dir}/target/{}", profile)
    } else {
      format!("{manifest_dir}/target/{target}/{}", profile)
    };

    let source = PathBuf::from(&source_dir).join(source_binary_name);
    if source.exists() {
      if let Err(e) = std::fs::create_dir_all(&binaries_dir) {
        eprintln!("cargo:warning=Failed to create binaries directory: {}", e);
        return;
      }
      if let Err(e) = std::fs::copy(&source, &binary_path) {
        eprintln!("cargo:warning=Failed to copy proxy binary: {}", e);
      }
    } else {
      eprintln!("cargo:warning=Proxy binary not found at {} and source {} doesn't exist. Run 'pnpm copy-proxy-binary' first.", binary_path.display(), source.display());
    }
  }
}
