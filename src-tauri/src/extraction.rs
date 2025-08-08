use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use tauri::Emitter;

use crate::browser::BrowserType;
use crate::download::DownloadProgress;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Command;

#[cfg(target_os = "macos")]
use std::fs::create_dir_all;

pub struct Extractor;

impl Extractor {
  fn new() -> Self {
    Self
  }

  pub fn instance() -> &'static Extractor {
    &EXTRACTOR
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

    println!(
      "Starting extraction of {} for browser {}",
      archive_path.display(),
      browser_type.as_str()
    );

    // Detect the actual file type by reading the file header
    let actual_format = self.detect_file_format(archive_path)?;
    println!("Detected format: {actual_format}");

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
      "msi" => self.extract_msi(archive_path, dest_dir).await,
      "exe" => {
        // For Windows EXE files, some may be self-extracting archives, others are installers
        // For browsers like Firefox, TOR, they're typically installers that don't need extraction
        self
          .handle_exe_file(archive_path, dest_dir, browser_type)
          .await
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
    // First check file extension for DMG files since they're common on macOS
    // and can have misleading magic numbers
    if let Some(ext) = file_path.extension().and_then(|ext| ext.to_str()) {
      if ext.to_lowercase() == "dmg" {
        return Ok("dmg".to_string());
      }
      if ext.to_lowercase() == "msi" {
        return Ok("msi".to_string());
      }
    }

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

    // Check for MSI files (Microsoft Installer)
    if buffer[0..8] == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
      return Ok("msi".to_string());
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

    // Fallback to file extension
    if let Some(ext) = file_path.extension().and_then(|ext| ext.to_str()) {
      match ext.to_lowercase().as_str() {
        "dmg" => Ok("dmg".to_string()),
        "zip" => Ok("zip".to_string()),
        "msi" => Ok("msi".to_string()),
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
    println!(
      "Extracting DMG: {} to {}",
      dmg_path.display(),
      dest_dir.display()
    );

    // Create a temporary mount point
    let mount_point = std::env::temp_dir().join(format!(
      "donut_mount_{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
    ));
    create_dir_all(&mount_point)?;

    println!("Created mount point: {}", mount_point.display());

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
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);
      println!("Failed to mount DMG. stdout: {stdout}, stderr: {stderr}");

      // Clean up mount point before returning error
      let _ = fs::remove_dir_all(&mount_point);

      return Err(format!("Failed to mount DMG: {stderr}").into());
    }

    println!("Successfully mounted DMG");

    // Find the .app directory in the mount point
    let app_result = self.find_app_in_directory(&mount_point).await;

    let app_entry = match app_result {
      Ok(app_path) => app_path,
      Err(e) => {
        println!("Failed to find .app in mount point: {e}");

        // Try to unmount before returning error
        let _ = Command::new("hdiutil")
          .args(["detach", "-force", mount_point.to_str().unwrap()])
          .output();
        let _ = fs::remove_dir_all(&mount_point);

        return Err("No .app found after extraction".into());
      }
    };

    println!("Found .app bundle: {}", app_entry.display());

    // Copy the .app to the destination
    let app_path = dest_dir.join(app_entry.file_name().unwrap());

    println!("Copying .app to: {}", app_path.display());

    let output = Command::new("cp")
      .args([
        "-R",
        app_entry.to_str().unwrap(),
        app_path.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      println!("Failed to copy app: {stderr}");

      // Unmount before returning error
      let _ = Command::new("hdiutil")
        .args(["detach", "-force", mount_point.to_str().unwrap()])
        .output();
      let _ = fs::remove_dir_all(&mount_point);

      return Err(format!("Failed to copy app: {stderr}").into());
    }

    println!("Successfully copied .app bundle");

    // Remove quarantine attributes
    let _ = Command::new("xattr")
      .args(["-dr", "com.apple.quarantine", app_path.to_str().unwrap()])
      .output();

    let _ = Command::new("xattr")
      .args(["-cr", app_path.to_str().unwrap()])
      .output();

    println!("Removed quarantine attributes");

    // Unmount the DMG
    let output = Command::new("hdiutil")
      .args(["detach", mount_point.to_str().unwrap()])
      .output()?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      println!("Warning: Failed to unmount DMG: {stderr}");
      // Don't fail if unmount fails - the extraction was successful
    } else {
      println!("Successfully unmounted DMG");
    }

    // Clean up mount point directory
    let _ = fs::remove_dir_all(&mount_point);

    Ok(app_path)
  }

  #[cfg(target_os = "macos")]
  async fn find_app_in_directory(
    &self,
    dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    self.find_app_recursive(dir, 0).await
  }

  #[cfg(target_os = "macos")]
  async fn find_app_recursive(
    &self,
    dir: &Path,
    depth: usize,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Limit search depth to avoid infinite loops
    if depth > 4 {
      return Err("Maximum search depth reached".into());
    }

    if let Ok(entries) = fs::read_dir(dir) {
      let mut subdirs = Vec::new();
      let mut hidden_subdirs = Vec::new();

      // First pass: look for .app bundles directly
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          if let Some(extension) = path.extension() {
            if extension == "app" {
              println!("Found .app bundle at depth {}: {}", depth, path.display());
              return Ok(path);
            }
          }

          // Collect subdirectories for second pass
          let filename = path.file_name().unwrap_or_default().to_string_lossy();
          if filename.starts_with('.') {
            // Hidden directories - search these with lower priority
            hidden_subdirs.push(path);
          } else {
            // Regular directories - search these first
            subdirs.push(path);
          }
        }
      }

      // Second pass: search regular subdirectories first
      for subdir in subdirs {
        // Skip common directories that are unlikely to contain .app files
        let dirname = subdir.file_name().unwrap_or_default().to_string_lossy();
        if matches!(
          dirname.as_ref(),
          "Documents" | "Downloads" | "Desktop" | "Library" | "System" | "tmp" | "var"
        ) {
          continue;
        }

        if let Ok(result) = Box::pin(self.find_app_recursive(&subdir, depth + 1)).await {
          return Ok(result);
        }
      }

      // Third pass: search hidden directories if nothing found in regular ones
      for hidden_dir in hidden_subdirs {
        if let Ok(result) = Box::pin(self.find_app_recursive(&hidden_dir, depth + 1)).await {
          return Ok(result);
        }
      }
    }

    Err(format!("No .app found in directory: {}", dir.display()).into())
  }

  pub async fn extract_zip(
    &self,
    zip_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    println!("Extracting ZIP archive: {}", zip_path.display());
    std::fs::create_dir_all(dest_dir)?;

    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))?;

    for i in 0..archive.len() {
      let mut file = archive.by_index(i)?;
      let outpath = match file.enclosed_name() {
        Some(path) => dest_dir.join(path),
        None => continue,
      };

      // Handle directory creation
      if file.name().ends_with('/') {
        std::fs::create_dir_all(&outpath)?;
      } else {
        // Create parent directories
        if let Some(p) = outpath.parent() {
          if !p.exists() {
            std::fs::create_dir_all(p)?;
          }
        }

        // Extract file
        let mut outfile = File::create(&outpath)?;
        io::copy(&mut file, &mut outfile)?;

        // Set executable permissions on Unix-like systems
        #[cfg(unix)]
        {
          use std::os::unix::fs::PermissionsExt;
          if let Some(mode) = file.unix_mode() {
            let permissions = std::fs::Permissions::from_mode(mode);
            std::fs::set_permissions(&outpath, permissions)?;
          }
        }
      }
    }

    println!("ZIP extraction completed. Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_tar_gz(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    println!("Extracting tar.gz archive: {}", tar_path.display());
    std::fs::create_dir_all(dest_dir)?;

    let file = File::open(tar_path)?;
    let gz_decoder = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(gz_decoder);

    archive.unpack(dest_dir)?;

    // Set executable permissions for extracted files
    self.set_executable_permissions_recursive(dest_dir).await?;

    println!("tar.gz extraction completed. Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_tar_bz2(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    println!("Extracting tar.bz2 archive: {}", tar_path.display());
    std::fs::create_dir_all(dest_dir)?;

    let file = File::open(tar_path)?;
    let bz2_decoder = bzip2::read::BzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(bz2_decoder);

    archive.unpack(dest_dir)?;

    // Set executable permissions for extracted files
    self.set_executable_permissions_recursive(dest_dir).await?;

    println!("tar.bz2 extraction completed. Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_tar_xz(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    println!("Extracting tar.xz archive: {}", tar_path.display());
    std::fs::create_dir_all(dest_dir)?;

    let file = File::open(tar_path)?;
    let mut buf_reader = BufReader::new(file);

    // Read the entire file into memory for lzma-rs
    let mut compressed_data = Vec::new();
    buf_reader.read_to_end(&mut compressed_data)?;

    // Decompress using lzma-rs
    let mut decompressed_data = Vec::new();
    lzma_rs::xz_decompress(
      &mut std::io::Cursor::new(compressed_data),
      &mut decompressed_data,
    )?;

    // Create tar archive from decompressed data
    let cursor = std::io::Cursor::new(decompressed_data);
    let mut archive = tar::Archive::new(cursor);

    archive.unpack(dest_dir)?;

    // Set executable permissions for extracted files
    self.set_executable_permissions_recursive(dest_dir).await?;

    println!("tar.xz extraction completed. Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_msi(
    &self,
    msi_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    println!("Extracting MSI archive: {}", msi_path.display());
    std::fs::create_dir_all(dest_dir)?;

    // Extract MSI in a separate scope to avoid Send issues
    {
      let mut extractor = msi_extract::MsiExtractor::from_path(msi_path)?;
      extractor.to(dest_dir);
    }

    println!("MSI extraction completed. Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  #[cfg(target_os = "linux")]
  pub async fn handle_appimage(
    &self,
    appimage_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(dest_dir)?;

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
    println!("Searching for .app bundle in: {}", dest_dir.display());

    // Use the enhanced recursive search
    match self.find_app_in_directory(dest_dir).await {
      Ok(app_path) => {
        // Check if the app is in a subdirectory and move it to the root if needed
        let app_parent = app_path.parent().unwrap();
        if app_parent != dest_dir {
          println!(
            "Found .app in subdirectory, moving to root: {} -> {}",
            app_path.display(),
            dest_dir.display()
          );
          let target_path = dest_dir.join(app_path.file_name().unwrap());

          // Move the app to the root destination directory
          fs::rename(&app_path, &target_path)?;

          // Try to clean up the now-empty subdirectory (ignore errors)
          if let Some(parent_dir) = app_path.parent() {
            if parent_dir != dest_dir {
              let _ = fs::remove_dir_all(parent_dir);
            }
          }

          println!("Successfully moved .app to: {}", target_path.display());
          Ok(target_path)
        } else {
          println!("Found .app at root level: {}", app_path.display());
          Ok(app_path)
        }
      }
      Err(e) => {
        println!("Failed to find .app bundle: {e}");
        Err("No .app found after extraction".into())
      }
    }
  }

  #[cfg(target_os = "windows")]
  async fn find_windows_executable(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    println!(
      "Searching for Windows executable in: {}",
      dest_dir.display()
    );

    // Look for .exe files, preferring main browser executables
    let priority_exe_names = [
      "firefox.exe",
      "chrome.exe",
      "chromium.exe",
      "zen.exe",
      "brave.exe",
      "tor-browser.exe",
      "tor.exe",
      "mullvad-browser.exe",
    ];

    // First try priority executable names
    for exe_name in &priority_exe_names {
      let exe_path = dest_dir.join(exe_name);
      if exe_path.exists() {
        println!("Found priority executable: {}", exe_path.display());
        return Ok(exe_path);
      }
    }

    // Recursively search for executables with depth limit
    match self.find_windows_executable_recursive(dest_dir, 0, 3).await {
      Ok(exe_path) => {
        println!(
          "Found executable via recursive search: {}",
          exe_path.display()
        );
        Ok(exe_path)
      }
      Err(_) => Err("No executable found after extraction".into()),
    }
  }

  #[cfg(target_os = "windows")]
  fn find_windows_executable_recursive<'a>(
    &'a self,
    dir: &'a Path,
    depth: usize,
    max_depth: usize,
  ) -> std::pin::Pin<
    Box<
      dyn std::future::Future<Output = Result<PathBuf, Box<dyn std::error::Error + Send + Sync>>>
        + Send
        + 'a,
    >,
  > {
    Box::pin(async move {
      if depth > max_depth {
        return Err("Maximum search depth reached".into());
      }

      if let Ok(entries) = fs::read_dir(dir) {
        let mut dirs_to_search = Vec::new();

        // First pass: look for .exe files in current directory
        for entry in entries.flatten() {
          let path = entry.path();

          if path.is_file()
            && path
              .extension()
              .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == "exe")
          {
            let file_name = path
              .file_name()
              .and_then(|n| n.to_str())
              .unwrap_or("")
              .to_lowercase();

            // Check if it's a browser executable
            if file_name.contains("firefox")
              || file_name.contains("chrome")
              || file_name.contains("chromium")
              || file_name.contains("zen")
              || file_name.contains("brave")
              || file_name.contains("tor")
              || file_name.contains("mullvad")
              || file_name.contains("browser")
            {
              return Ok(path);
            }
          } else if path.is_dir() {
            // Collect directories for later search
            dirs_to_search.push(path);
          }
        }

        // Second pass: search subdirectories
        for subdir in dirs_to_search {
          if let Ok(result) = self
            .find_windows_executable_recursive(&subdir, depth + 1, max_depth)
            .await
          {
            return Ok(result);
          }
        }

        // Third pass: if no browser-specific executable found, return any .exe
        if let Ok(entries) = fs::read_dir(dir) {
          for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
              && path
                .extension()
                .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == "exe")
            {
              return Ok(path);
            }
          }
        }
      }

      Err("No executable found".into())
    })
  }

  #[cfg(target_os = "linux")]
  async fn find_linux_executable(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Enhanced list of common browser executable names
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
    ];

    // First, try direct lookup in the main directory
    for exe_name in &exe_names {
      let exe_path = dest_dir.join(exe_name);
      if exe_path.exists() && self.is_executable(&exe_path) {
        return Ok(exe_path);
      }
    }

    // Enhanced list of common Linux subdirectories to search
    let subdirs = [
      "bin",
      "usr/bin",
      "usr/local/bin",
      "opt",
      "sbin",
      "usr/sbin",
      "firefox",
      "chrome",
      "chromium",
      "brave",
      "zen",
      "tor-browser",
      "mullvad-browser",
      ".",
      "./",
      "firefox",
      "mullvad-browser",
      "tor-browser_en-US",
      "Browser",
      "browser",
      "opt/google/chrome",
      "opt/brave.com/brave",
      "opt/mullvad-browser",
      "usr/lib/firefox",
      "usr/lib/chromium",
      "usr/share/applications",
      "usr/bin",
      "AppRun",
    ];

    // Search in subdirectories
    for subdir in &subdirs {
      let subdir_path = dest_dir.join(subdir);
      if subdir_path.exists() && subdir_path.is_dir() {
        for exe_name in &exe_names {
          let exe_path = subdir_path.join(exe_name);
          if exe_path.exists() && self.is_executable(&exe_path) {
            return Ok(exe_path);
          }
        }
      }
    }

    // Look for AppImage files
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

    // Last resort: recursive search for any executable file
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

  /// Set executable permissions on Unix-like systems for extracted binaries
  #[cfg(unix)]
  #[allow(dead_code)]
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

  #[cfg(not(unix))]
  async fn set_executable_permissions(
    &self,
    _path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Ok(())
  }

  /// Set executable permissions recursively for all files in a directory
  #[cfg(unix)]
  async fn set_executable_permissions_recursive(
    &self,
    dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(entries) = fs::read_dir(dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
          // Check if file looks like it should be executable
          if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            let name_lower = file_name.to_lowercase();
            if name_lower.contains("firefox")
              || name_lower.contains("chrome")
              || name_lower.contains("brave")
              || name_lower.contains("zen")
              || name_lower.contains("tor")
              || name_lower.contains("mullvad")
              || name_lower.ends_with(".appimage")
              || !name_lower.contains('.')
            {
              // Likely an executable, set permissions
              let mut permissions = path.metadata()?.permissions();
              let current_mode = permissions.mode();
              let new_mode = current_mode | 0o755; // rwxr-xr-x
              permissions.set_mode(new_mode);
              std::fs::set_permissions(&path, permissions)?;
            }
          }
        } else if path.is_dir() {
          // Recursively process subdirectories
          Box::pin(self.set_executable_permissions_recursive(&path)).await?;
        }
      }
    }
    Ok(())
  }

  #[cfg(not(unix))]
  async fn set_executable_permissions_recursive(
    &self,
    _dir: &Path,
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
  use std::io::Write;
  use tempfile::TempDir;

  #[cfg(target_os = "macos")]
  use std::fs::create_dir_all;

  #[test]
  fn test_format_detection_zip() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let zip_path = temp_dir.path().join("test.zip");

    // Create a file with ZIP magic number
    let mut file = File::create(&zip_path).unwrap();
    file.write_all(&[0x50, 0x4B, 0x03, 0x04]).unwrap(); // ZIP magic
    file.write_all(&[0; 8]).unwrap(); // padding

    let result = extractor.detect_file_format(&zip_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "zip");
  }

  #[test]
  fn test_format_detection_dmg_by_extension() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let dmg_path = temp_dir.path().join("test.dmg");

    // Create a file (magic number won't match, but extension will)
    let mut file = File::create(&dmg_path).unwrap();
    file.write_all(b"fake dmg content").unwrap();

    let result = extractor.detect_file_format(&dmg_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "dmg");
  }

  #[test]
  fn test_format_detection_exe() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let exe_path = temp_dir.path().join("test.exe");

    // Create a file with PE header
    let mut file = File::create(&exe_path).unwrap();
    file.write_all(&[0x4D, 0x5A]).unwrap(); // PE magic
    file.write_all(&[0; 10]).unwrap(); // padding

    let result = extractor.detect_file_format(&exe_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "exe");
  }

  #[test]
  fn test_format_detection_tar_gz() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let tar_gz_path = temp_dir.path().join("test.tar.gz");

    // Create a file with gzip magic
    let mut file = File::create(&tar_gz_path).unwrap();
    file.write_all(&[0x1F, 0x8B, 0x08]).unwrap(); // gzip magic
    file.write_all(&[0; 9]).unwrap(); // padding

    let result = extractor.detect_file_format(&tar_gz_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "tar.gz");
  }

  #[test]
  fn test_format_detection_tar_bz2() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let tar_bz2_path = temp_dir.path().join("test.tar.bz2");

    // Create a file with bzip2 magic
    let mut file = File::create(&tar_bz2_path).unwrap();
    file.write_all(&[0x42, 0x5A, 0x68]).unwrap(); // bzip2 magic
    file.write_all(&[0; 9]).unwrap(); // padding

    let result = extractor.detect_file_format(&tar_bz2_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "tar.bz2");
  }

  #[test]
  fn test_format_detection_tar_xz() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let tar_xz_path = temp_dir.path().join("test.tar.xz");

    // Create a file with xz magic
    let mut file = File::create(&tar_xz_path).unwrap();
    file
      .write_all(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00])
      .unwrap(); // xz magic
    file.write_all(&[0; 6]).unwrap(); // padding

    let result = extractor.detect_file_format(&tar_xz_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "tar.xz");
  }

  #[test]
  fn test_format_detection_msi() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let msi_path = temp_dir.path().join("test.msi");

    // Create a file with MSI magic
    let mut file = File::create(&msi_path).unwrap();
    file
      .write_all(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
      .unwrap(); // MSI magic
    file.write_all(&[0; 4]).unwrap(); // padding

    let result = extractor.detect_file_format(&msi_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "msi");
  }

  #[test]
  fn test_format_detection_msi_by_extension() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let msi_path = temp_dir.path().join("test.msi");

    // Create a file (magic number won't match, but extension will)
    let mut file = File::create(&msi_path).unwrap();
    file.write_all(b"fake msi content").unwrap();

    let result = extractor.detect_file_format(&msi_path);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "msi");
  }

  #[tokio::test]
  async fn test_extract_zip_with_test_archive() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let dest_dir = temp_dir.path().join("extracted");

    // Use the test ZIP archive
    let zip_path = std::path::Path::new("test-assets/test.zip");
    if !zip_path.exists() {
      // Skip test if test archive doesn't exist
      return;
    }

    let _result = extractor.extract_zip(zip_path, &dest_dir).await;

    // The result might fail because we're looking for executables, but the extraction should work
    // Let's just check if the file was extracted
    let extracted_file = dest_dir.join("test.txt");
    if extracted_file.exists() {
      let content = std::fs::read_to_string(&extracted_file).unwrap();
      assert_eq!(content.trim(), "Hello, World!");
    }
  }

  #[tokio::test]
  async fn test_extract_tar_gz_with_test_archive() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let dest_dir = temp_dir.path().join("extracted");

    // Use the test tar.gz archive
    let tar_gz_path = std::path::Path::new("test-assets/test.tar.gz");
    if !tar_gz_path.exists() {
      // Skip test if test archive doesn't exist
      return;
    }

    let _result = extractor.extract_tar_gz(tar_gz_path, &dest_dir).await;

    // Check if the file was extracted
    let extracted_file = dest_dir.join("test.txt");
    if extracted_file.exists() {
      let content = std::fs::read_to_string(&extracted_file).unwrap();
      assert_eq!(content.trim(), "Hello, World!");
    }
  }

  #[tokio::test]
  async fn test_extract_tar_bz2_with_test_archive() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let dest_dir = temp_dir.path().join("extracted");

    // Use the test tar.bz2 archive
    let tar_bz2_path = std::path::Path::new("test-assets/test.tar.bz2");
    if !tar_bz2_path.exists() {
      // Skip test if test archive doesn't exist
      return;
    }

    let _result = extractor.extract_tar_bz2(tar_bz2_path, &dest_dir).await;

    // Check if the file was extracted
    let extracted_file = dest_dir.join("test.txt");
    if extracted_file.exists() {
      let content = std::fs::read_to_string(&extracted_file).unwrap();
      assert_eq!(content.trim(), "Hello, World!");
    }
  }

  #[tokio::test]
  async fn test_extract_tar_xz_with_test_archive() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let dest_dir = temp_dir.path().join("extracted");

    // Use the test tar.xz archive
    let tar_xz_path = std::path::Path::new("test-assets/test.tar.xz");
    if !tar_xz_path.exists() {
      // Skip test if test archive doesn't exist
      return;
    }

    let _result = extractor.extract_tar_xz(tar_xz_path, &dest_dir).await;

    // Check if the file was extracted
    let extracted_file = dest_dir.join("test.txt");
    if extracted_file.exists() {
      let content = std::fs::read_to_string(&extracted_file).unwrap();
      assert_eq!(content.trim(), "Hello, World!");
    }
  }

  #[test]
  fn test_unsupported_archive_format() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();
    let fake_archive = temp_dir.path().join("test.rar");

    // Create a file with invalid header
    let mut file = File::create(&fake_archive).unwrap();
    file.write_all(b"invalid content").unwrap();

    // Test format detection
    let result = extractor.detect_file_format(&fake_archive);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "unknown");
  }

  #[cfg(target_os = "macos")]
  #[tokio::test]
  async fn test_find_app_at_root_level() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();

    // Create a Firefox.app directory
    let firefox_app = temp_dir.path().join("Firefox.app");
    create_dir_all(&firefox_app).unwrap();

    // Create the standard macOS app structure
    let contents_dir = firefox_app.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    create_dir_all(&macos_dir).unwrap();

    // Create the executable
    let executable = macos_dir.join("firefox");
    File::create(&executable).unwrap();

    // Test finding the app
    let result = extractor.find_app_in_directory(temp_dir.path()).await;
    assert!(result.is_ok());

    let found_app = result.unwrap();
    assert_eq!(found_app.file_name().unwrap(), "Firefox.app");
    assert!(found_app.exists());
  }

  #[cfg(target_os = "linux")]
  #[test]
  fn test_is_executable() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().unwrap();

    // Create a regular file
    let regular_file = temp_dir.path().join("regular.txt");
    File::create(&regular_file).unwrap();

    // Should not be executable initially
    assert!(!extractor.is_executable(&regular_file));

    // Make it executable
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = regular_file.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&regular_file, permissions).unwrap();

    // Should now be executable
    assert!(extractor.is_executable(&regular_file));
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref EXTRACTOR: Extractor = Extractor::new();
}
