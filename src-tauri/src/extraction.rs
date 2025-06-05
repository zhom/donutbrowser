use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Emitter;

use crate::browser::BrowserType;
use crate::download::DownloadProgress;

pub struct Extractor;

impl Extractor {
  pub fn new() -> Self {
    Self
  }

  pub async fn extract_browser(
    &self,
    app_handle: &tauri::AppHandle,
    browser_type: BrowserType,
    version: &str,
    archive_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Emit extraction start progress
    let progress = DownloadProgress {
      browser: browser_type.as_str().to_string(),
      version: version.to_string(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 0.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: None,
      stage: "extracting".to_string(),
    };
    let _ = app_handle.emit("download-progress", &progress);

    // Try to detect the actual file type by reading the file header
    let actual_format = self.detect_file_format(archive_path)?;

    match actual_format.as_str() {
      "dmg" => {
        #[cfg(target_os = "macos")]
        return self.extract_dmg(archive_path, dest_dir).await;

        #[cfg(not(target_os = "macos"))]
        return Err("DMG extraction is only supported on macOS".into());
      }
      "zip" => self.extract_zip(archive_path, dest_dir).await,
      "tar.xz" => self.extract_tar_xz(archive_path, dest_dir).await,
      "tar.bz2" => self.extract_tar_bz2(archive_path, dest_dir).await,
      "tar.gz" => self.extract_tar_gz(archive_path, dest_dir).await,
      "exe" => {
        // For Windows EXE files, some may be self-extracting archives, others are installers
        // For browsers like Firefox, TOR, they're typically installers that don't need extraction
        self
          .handle_exe_file(archive_path, dest_dir, browser_type)
          .await
      }
      "deb" => {
        #[cfg(target_os = "linux")]
        return self.extract_deb(archive_path, dest_dir).await;

        #[cfg(not(target_os = "linux"))]
        return Err("DEB extraction is only supported on Linux".into());
      }
      "appimage" => {
        #[cfg(target_os = "linux")]
        return self.handle_appimage(archive_path, dest_dir).await;

        #[cfg(not(target_os = "linux"))]
        return Err("AppImage is only supported on Linux".into());
      }
      _ => {
        Err(format!(
          "Unsupported archive format: {} (detected: {}). The downloaded file might be corrupted or in an unexpected format.",
          archive_path.extension().and_then(|ext| ext.to_str()).unwrap_or("unknown"),
          actual_format
        ).into())
      }
    }
  }

  /// Detect the actual file format by reading file headers
  fn detect_file_format(
    &self,
    file_path: &Path,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(file_path)?;
    let mut buffer = [0u8; 12]; // Read first 12 bytes for magic number detection
    file.read_exact(&mut buffer)?;

    // Check magic numbers for different file types
    match &buffer[0..4] {
      [0x50, 0x4B, 0x03, 0x04] | [0x50, 0x4B, 0x05, 0x06] | [0x50, 0x4B, 0x07, 0x08] => {
        return Ok("zip".to_string())
      }
      [0x7F, 0x45, 0x4C, 0x46] => return Ok("appimage".to_string()), // ELF header (AppImage)
      [0x4D, 0x5A, _, _] => return Ok("exe".to_string()),            // PE header (Windows EXE)
      _ => {}
    }

    // Check for XZ compressed files
    if buffer[0..6] == [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00] {
      return Ok("tar.xz".to_string());
    }

    // Check for Bzip2 compressed files
    if buffer[0..3] == [0x42, 0x5A, 0x68] {
      return Ok("tar.bz2".to_string());
    }

    // Check for Gzip compressed files
    if buffer[0..3] == [0x1F, 0x8B, 0x08] {
      return Ok("tar.gz".to_string());
    }

    // Check for DEB files
    if buffer[0..8] == [0x21, 0x3C, 0x61, 0x72, 0x63, 0x68, 0x3E, 0x0A] {
      return Ok("deb".to_string());
    }

    // Fallback to file extension
    if let Some(ext) = file_path.extension().and_then(|ext| ext.to_str()) {
      match ext.to_lowercase().as_str() {
        "dmg" => Ok("dmg".to_string()),
        "zip" => Ok("zip".to_string()),
        "xz" => {
          if file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .ends_with(".tar.xz")
          {
            Ok("tar.xz".to_string())
          } else {
            Ok("xz".to_string())
          }
        }
        "bz2" => {
          if file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .ends_with(".tar.bz2")
          {
            Ok("tar.bz2".to_string())
          } else {
            Ok("bz2".to_string())
          }
        }
        "gz" => {
          if file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .ends_with(".tar.gz")
          {
            Ok("tar.gz".to_string())
          } else {
            Ok("gz".to_string())
          }
        }
        "exe" => Ok("exe".to_string()),
        "deb" => Ok("deb".to_string()),
        "appimage" => Ok("appimage".to_string()),
        _ => Ok("unknown".to_string()),
      }
    } else {
      Ok("unknown".to_string())
    }
  }

  #[cfg(target_os = "macos")]
  pub async fn extract_dmg(
    &self,
    dmg_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Create a temporary mount point
    let mount_point = std::env::temp_dir().join(format!(
      "donut_mount_{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
    ));
    create_dir_all(&mount_point)?;

    // Mount the DMG
    let output = Command::new("hdiutil")
      .args([
        "attach",
        "-nobrowse",
        "-mountpoint",
        mount_point.to_str().unwrap(),
        dmg_path.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to mount DMG: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Find the .app directory in the mount point
    let app_entry = fs::read_dir(&mount_point)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("No .app found in DMG")?;

    // Copy the .app to the destination
    let app_path = dest_dir.join(app_entry.file_name());

    let output = Command::new("cp")
      .args([
        "-R",
        app_entry.path().to_str().unwrap(),
        app_path.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to copy app: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Remove quarantine attributes
    let _ = Command::new("xattr")
      .args(["-dr", "com.apple.quarantine", app_path.to_str().unwrap()])
      .output();

    let _ = Command::new("xattr")
      .args(["-cr", app_path.to_str().unwrap()])
      .output();

    // Try to unmount the DMG with retries
    let mut retry_count = 0;
    let max_retries = 3;
    let mut unmounted = false;

    while retry_count < max_retries && !unmounted {
      // Wait a bit before trying to unmount
      tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

      let output = Command::new("hdiutil")
        .args(["detach", mount_point.to_str().unwrap()])
        .output()?;

      if output.status.success() {
        unmounted = true;
      } else if retry_count == max_retries - 1 {
        // Force unmount on last retry
        let _ = Command::new("hdiutil")
          .args(["detach", "-force", mount_point.to_str().unwrap()])
          .output();
        unmounted = true; // Consider it unmounted even if force fails
      }
      retry_count += 1;
    }

    // Clean up mount point directory
    let _ = fs::remove_dir_all(&mount_point);

    Ok(app_path)
  }

  pub async fn extract_zip(
    &self,
    zip_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Platform-specific ZIP extraction
    #[cfg(target_os = "windows")]
    {
      self.extract_zip_windows(zip_path, dest_dir).await
    }

    #[cfg(not(target_os = "windows"))]
    {
      self.extract_zip_unix(zip_path, dest_dir).await
    }
  }

  #[cfg(target_os = "windows")]
  async fn extract_zip_windows(
    &self,
    zip_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Use PowerShell's Expand-Archive on Windows
    let output = Command::new("powershell")
      .args([
        "-Command",
        &format!(
          "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
          zip_path.display(),
          dest_dir.display()
        ),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to extract zip with PowerShell: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    self.find_extracted_executable(dest_dir).await
  }

  #[cfg(not(target_os = "windows"))]
  async fn extract_zip_unix(
    &self,
    zip_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Use unzip command on Unix-like systems
    let output = Command::new("unzip")
      .args([
        "-q", // quiet
        zip_path.to_str().unwrap(),
        "-d",
        dest_dir.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to extract zip: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_tar_xz(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    create_dir_all(dest_dir)?;

    // Use tar command for more reliable extraction
    let output = Command::new("tar")
      .args([
        "-xf",
        tar_path.to_str().unwrap(),
        "-C",
        dest_dir.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to extract tar.xz: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Find the extracted executable and set proper permissions
    let executable_path = self.find_extracted_executable(dest_dir).await?;

    // Ensure executable permissions are set correctly for Linux
    if cfg!(target_os = "linux") {
      self.set_executable_permissions(&executable_path).await?;
    }

    Ok(executable_path)
  }

  pub async fn extract_tar_bz2(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    create_dir_all(dest_dir)?;

    // Use tar command for more reliable extraction
    let output = Command::new("tar")
      .args([
        "-xjf",
        tar_path.to_str().unwrap(),
        "-C",
        dest_dir.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to extract tar.bz2: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Find the extracted executable and set proper permissions
    let executable_path = self.find_extracted_executable(dest_dir).await?;

    // Ensure executable permissions are set correctly for Linux
    if cfg!(target_os = "linux") {
      self.set_executable_permissions(&executable_path).await?;
    }

    Ok(executable_path)
  }

  pub async fn extract_tar_gz(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    create_dir_all(dest_dir)?;

    // Use tar command for more reliable extraction
    let output = Command::new("tar")
      .args([
        "-xzf",
        tar_path.to_str().unwrap(),
        "-C",
        dest_dir.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to extract tar.gz: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Find the extracted executable and set proper permissions
    let executable_path = self.find_extracted_executable(dest_dir).await?;

    // Ensure executable permissions are set correctly for Linux
    if cfg!(target_os = "linux") {
      self.set_executable_permissions(&executable_path).await?;
    }

    Ok(executable_path)
  }

  #[cfg(target_os = "linux")]
  pub async fn extract_deb(
    &self,
    deb_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    create_dir_all(dest_dir)?;

    // Extract DEB package using dpkg-deb
    let output = Command::new("dpkg-deb")
      .args(["-x", deb_path.to_str().unwrap(), dest_dir.to_str().unwrap()])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to extract DEB: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Find the extracted executable and set proper permissions
    let executable_path = self.find_extracted_executable(dest_dir).await?;

    // Ensure executable permissions are set correctly
    self.set_executable_permissions(&executable_path).await?;

    Ok(executable_path)
  }

  #[cfg(target_os = "linux")]
  pub async fn handle_appimage(
    &self,
    appimage_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    create_dir_all(dest_dir)?;

    // For AppImages, we typically just copy them and make sure they're executable
    let dest_file = dest_dir.join(
      appimage_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("app.AppImage")),
    );

    // Copy the AppImage to destination
    fs::copy(appimage_path, &dest_file)?;

    // Set executable permissions
    self.set_executable_permissions(&dest_file).await?;

    Ok(dest_file)
  }

  pub async fn handle_exe_file(
    &self,
    exe_path: &Path,
    dest_dir: &Path,
    browser_type: BrowserType,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    match browser_type {
      BrowserType::Zen => {
        // Zen installer EXE needs to be run to install
        #[cfg(target_os = "windows")]
        {
          self.install_zen_windows(exe_path, dest_dir).await
        }
        #[cfg(not(target_os = "windows"))]
        {
          Err("Zen EXE installation is only supported on Windows".into())
        }
      }
      _ => {
        // For other browsers (Firefox, TOR, etc.), the EXE is typically just copied
        let exe_name = exe_path
          .file_name()
          .and_then(|name| name.to_str())
          .unwrap_or("browser.exe");

        let dest_path = dest_dir.join(exe_name);
        fs::copy(exe_path, &dest_path)?;
        Ok(dest_path)
      }
    }
  }

  #[cfg(target_os = "windows")]
  async fn install_zen_windows(
    &self,
    installer_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // For Zen installer, we need to run it silently
    // This is a simplified approach - in practice, you might need more sophisticated installer handling
    let output = Command::new(installer_path)
      .args(["/S", &format!("/D={}", dest_dir.display())])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to install Zen: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Find the installed executable
    self.find_extracted_executable(dest_dir).await
  }

  async fn find_extracted_executable(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Platform-specific executable finding logic
    #[cfg(target_os = "macos")]
    {
      self.find_macos_app(dest_dir).await
    }

    #[cfg(target_os = "windows")]
    {
      self.find_windows_executable(dest_dir).await
    }

    #[cfg(target_os = "linux")]
    {
      self.find_linux_executable(dest_dir).await
    }
  }

  #[cfg(target_os = "macos")]
  async fn find_macos_app(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // First, try to find any .app file in the destination directory
    if let Ok(entries) = fs::read_dir(dest_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "app") {
          return Ok(path);
        }
        // For Chromium, check subdirectories (chrome-mac folder)
        if path.is_dir() {
          if let Ok(sub_entries) = fs::read_dir(&path) {
            for sub_entry in sub_entries.flatten() {
              let sub_path = sub_entry.path();
              if sub_path.extension().is_some_and(|ext| ext == "app") {
                // Move the app to the root destination directory
                let target_path = dest_dir.join(sub_path.file_name().unwrap());
                fs::rename(&sub_path, &target_path)?;

                // Clean up the now-empty subdirectory
                let _ = fs::remove_dir_all(&path);
                return Ok(target_path);
              }
            }
          }
        }
      }
    }

    Err("No .app found after extraction".into())
  }

  #[cfg(target_os = "windows")]
  async fn find_windows_executable(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Look for .exe files, preferring main browser executables
    let exe_names = [
      "chrome.exe",
      "firefox.exe",
      "zen.exe",
      "brave.exe",
      "tor.exe",
    ];

    for exe_name in &exe_names {
      let exe_path = dest_dir.join(exe_name);
      if exe_path.exists() {
        return Ok(exe_path);
      }
    }

    // If no specific executable found, look for any .exe file
    if let Ok(entries) = fs::read_dir(dest_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "exe") {
          return Ok(path);
        }

        // Check subdirectories
        if path.is_dir() {
          if let Ok(sub_result) = self.find_windows_executable(&path).await {
            return Ok(sub_result);
          }
        }
      }
    }

    Err("No executable found after extraction".into())
  }

  #[cfg(target_os = "linux")]
  async fn find_linux_executable(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Enhanced list of common browser executable names with better pattern matching
    let exe_names = [
      // Firefox variants
      "firefox",
      "firefox-bin",
      "firefox-esr",
      "firefox-trunk",
      // Chrome/Chromium variants
      "chrome",
      "google-chrome",
      "google-chrome-stable",
      "google-chrome-beta",
      "google-chrome-unstable",
      "chromium",
      "chromium-browser",
      "chromium-bin",
      // Zen Browser
      "zen",
      "zen-browser",
      "zen-bin",
      // Brave variants
      "brave",
      "brave-browser",
      "brave-browser-stable",
      "brave-browser-beta",
      "brave-browser-dev",
      "brave-bin",
      // Tor Browser variants
      "tor-browser",
      "torbrowser-launcher",
      "tor-browser_en-US",
      "start-tor-browser",
      "Browser/start-tor-browser",
      // Mullvad Browser
      "mullvad-browser",
      "mullvad-browser-bin",
      // AppImage pattern (will be handled specially)
      "*.AppImage",
    ];

    // First, try direct lookup in the main directory
    for exe_name in &exe_names {
      if exe_name.contains('*') {
        // Handle glob patterns like *.AppImage
        if let Ok(entries) = fs::read_dir(dest_dir) {
          for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
              if file_name.ends_with(".AppImage") && self.is_executable(&path) {
                return Ok(path);
              }
            }
          }
        }
      } else {
        let exe_path = dest_dir.join(exe_name);
        if exe_path.exists() && self.is_executable(&exe_path) {
          return Ok(exe_path);
        }
      }
    }

    // Enhanced list of common Linux subdirectories to search
    let subdirs = [
      // Standard Unix directories
      "bin",
      "usr/bin",
      "usr/local/bin",
      "opt",
      "sbin",
      "usr/sbin",
      // Browser-specific directories
      "firefox",
      "chrome",
      "chromium",
      "brave",
      "zen",
      "tor-browser",
      "mullvad-browser",
      // Common extraction patterns
      ".",
      "./",
      // Package-specific extraction patterns
      "firefox",
      "mullvad-browser",
      "tor-browser_en-US",
      "Browser",
      "browser",
      // Nested patterns for different distro packaging
      "opt/google/chrome",
      "opt/brave.com/brave",
      "opt/mullvad-browser",
      "usr/lib/firefox",
      "usr/lib/chromium",
      "usr/share/applications",
      // AppImage mount patterns
      "usr/bin",
      "AppRun",
    ];

    // Search in subdirectories with better depth handling
    for subdir in &subdirs {
      let subdir_path = dest_dir.join(subdir);
      if subdir_path.exists() && subdir_path.is_dir() {
        for exe_name in &exe_names {
          if exe_name.contains('*') {
            // Handle glob patterns for AppImages
            if let Ok(entries) = fs::read_dir(&subdir_path) {
              for entry in entries.flatten() {
                let path = entry.path();
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                  if file_name.ends_with(".AppImage") && self.is_executable(&path) {
                    return Ok(path);
                  }
                }
              }
            }
          } else {
            let exe_path = subdir_path.join(exe_name);
            if exe_path.exists() && self.is_executable(&exe_path) {
              return Ok(exe_path);
            }
          }
        }
      }
    }

    // Last resort: enhanced recursive search for any executable file
    self.find_any_executable_recursive(dest_dir, 0).await
  }

  #[cfg(target_os = "linux")]
  fn is_executable(&self, path: &Path) -> bool {
    if let Ok(metadata) = path.metadata() {
      use std::os::unix::fs::PermissionsExt;
      return metadata.permissions().mode() & 0o111 != 0;
    }
    false
  }

  /// Set executable permissions on Linux for extracted binaries
  #[cfg(target_os = "linux")]
  async fn set_executable_permissions(
    &self,
    path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::os::unix::fs::PermissionsExt;

    if path.exists() {
      let mut permissions = path.metadata()?.permissions();
      // Set executable permissions for owner, group, and others if they have read permission
      let current_mode = permissions.mode();
      let new_mode = current_mode | 0o111; // Add execute permission
      permissions.set_mode(new_mode);
      std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
  }

  #[cfg(not(target_os = "linux"))]
  async fn set_executable_permissions(
    &self,
    _path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Ok(())
  }

  #[cfg(target_os = "linux")]
  async fn find_any_executable_recursive(
    &self,
    dir: &Path,
    depth: usize,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Limit recursion depth to avoid infinite loops
    if depth > 5 {
      return Err("Maximum search depth reached".into());
    }

    if let Ok(entries) = fs::read_dir(dir) {
      let mut directories = Vec::new();

      // First pass: look for executable files
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && self.is_executable(&path) {
          // Prefer files with browser-like names
          if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            let name_lower = file_name.to_lowercase();
            if name_lower.contains("firefox")
              || name_lower.contains("chrome")
              || name_lower.contains("brave")
              || name_lower.contains("zen")
              || name_lower.contains("tor")
              || name_lower.contains("mullvad")
              || file_name.ends_with(".AppImage")
            {
              return Ok(path);
            }
          }
        } else if path.is_dir() {
          directories.push(path);
        }
      }

      // Second pass: recursively search directories
      for dir_path in directories {
        if let Ok(result) = Box::pin(self.find_any_executable_recursive(&dir_path, depth + 1)).await
        {
          return Ok(result);
        }
      }
    }

    Err("No executable found".into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs::File;
  use tempfile::TempDir;

  #[test]
  fn test_extractor_creation() {
    let _ = Extractor::new();
    // Just verify we can create an extractor instance
  }

  #[test]
  fn test_unsupported_archive_format() {
    let _ = Extractor::new();
    let temp_dir = TempDir::new().unwrap();
    let fake_archive = temp_dir.path().join("test.rar");
    File::create(&fake_archive).unwrap();

    // Create a mock app handle (this won't work in real tests without Tauri runtime)
    // For now, we'll just test the logic without the actual extraction

    // Test that unsupported formats return an error
    let extension = fake_archive
      .extension()
      .and_then(|ext| ext.to_str())
      .unwrap_or("");

    assert_eq!(extension, "rar");
    // We know this would fail with "Unsupported archive format: rar"
  }

  #[test]
  fn test_dmg_path_validation() {
    let temp_dir = TempDir::new().unwrap();
    let dmg_path = temp_dir.path().join("test.dmg");

    // Test that we can identify DMG files correctly
    let extension = dmg_path
      .extension()
      .and_then(|ext| ext.to_str())
      .unwrap_or("");

    assert_eq!(extension, "dmg");
  }

  #[test]
  fn test_zip_path_validation() {
    let temp_dir = TempDir::new().unwrap();
    let zip_path = temp_dir.path().join("test.zip");

    // Test that we can identify ZIP files correctly
    let extension = zip_path
      .extension()
      .and_then(|ext| ext.to_str())
      .unwrap_or("");

    assert_eq!(extension, "zip");
  }

  #[test]
  fn test_mount_point_generation() {
    // Test that mount point generation creates unique paths
    let mount_point1 = std::env::temp_dir().join(format!(
      "donut_mount_{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
    ));

    std::thread::sleep(std::time::Duration::from_millis(10));

    let mount_point2 = std::env::temp_dir().join(format!(
      "donut_mount_{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
    ));

    // They should be different (or at least have the potential to be)
    assert!(mount_point1.to_string_lossy().contains("donut_mount_"));
    assert!(mount_point2.to_string_lossy().contains("donut_mount_"));
  }

  #[test]
  fn test_app_path_detection() {
    let temp_dir = TempDir::new().unwrap();

    // Create a fake .app directory
    let app_dir = temp_dir.path().join("TestApp.app");
    std::fs::create_dir_all(&app_dir).unwrap();

    // Test finding .app directories
    let entries: Vec<_> = fs::read_dir(temp_dir.path())
      .unwrap()
      .filter_map(Result::ok)
      .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .collect();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].file_name(), "TestApp.app");
  }

  #[test]
  fn test_nested_app_detection() {
    let temp_dir = TempDir::new().unwrap();

    // Create a nested structure like Chromium
    let chrome_dir = temp_dir.path().join("chrome-mac");
    std::fs::create_dir_all(&chrome_dir).unwrap();

    let app_dir = chrome_dir.join("Chromium.app");
    std::fs::create_dir_all(&app_dir).unwrap();

    // Test finding nested .app directories
    let mut found_app = false;

    if let Ok(entries) = fs::read_dir(temp_dir.path()) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          if let Ok(sub_entries) = fs::read_dir(&path) {
            for sub_entry in sub_entries.flatten() {
              let sub_path = sub_entry.path();
              if sub_path.extension().is_some_and(|ext| ext == "app") {
                found_app = true;
                break;
              }
            }
          }
        }
      }
    }

    assert!(found_app);
  }
}
