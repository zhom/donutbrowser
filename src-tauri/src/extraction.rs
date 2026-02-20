use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::browser::BrowserType;
use crate::downloader::DownloadProgress;
use crate::events;

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

  // NOTE: We intentionally do not rename or sanitize ZIP entry paths.
  // We only ensure paths are enclosed within the destination using zip's enclosed_name.

  /// Ensure the extracted files are in the correct directory structure expected by verification
  #[cfg(target_os = "linux")]
  async fn ensure_correct_directory_structure(
    &self,
    dest_dir: &Path,
    exe_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Determine browser type from the destination directory path
    let browser_type = if dest_dir.to_string_lossy().contains("camoufox") {
      "camoufox"
    } else if dest_dir.to_string_lossy().contains("wayfern") {
      "wayfern"
    } else if dest_dir.to_string_lossy().contains("firefox") {
      "firefox"
    } else if dest_dir.to_string_lossy().contains("zen") {
      "zen"
    } else {
      // For other browsers, assume the structure is already correct
      return Ok(());
    };

    // For Camoufox and Wayfern on Linux, we expect the executable directly under version directory
    // e.g., binaries/camoufox/<version>/camoufox, without an extra subdirectory
    if browser_type == "camoufox" || browser_type == "wayfern" {
      return Ok(());
    }

    let expected_subdir = dest_dir.join(browser_type);

    // If the executable is not in the expected subdirectory, create the structure
    if !exe_path.starts_with(&expected_subdir) {
      log::info!("Reorganizing directory structure for {}", browser_type);

      // Create the expected subdirectory
      std::fs::create_dir_all(&expected_subdir)?;

      // Move all files from the root to the subdirectory
      if let Ok(entries) = std::fs::read_dir(dest_dir) {
        for entry in entries.flatten() {
          let path = entry.path();
          let file_name = match path.file_name() {
            Some(name) => name,
            None => continue,
          };

          // Skip the subdirectory we just created
          if path == expected_subdir {
            continue;
          }

          let target_path = expected_subdir.join(file_name);

          // Move the file/directory
          if let Err(e) = std::fs::rename(&path, &target_path) {
            log::info!(
              "Warning: Failed to move {} to {}: {}",
              path.display(),
              target_path.display(),
              e
            );
          } else {
            log::info!("Moved {} to {}", path.display(), target_path.display());
          }
        }
      }

      log::info!("Directory structure reorganized for {}", browser_type);
    }

    Ok(())
  }

  pub async fn extract_browser(
    &self,
    _app_handle: &tauri::AppHandle,
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
    let _ = events::emit("download-progress", &progress);

    log::info!(
      "Starting extraction of {} for browser {} version {}",
      archive_path.display(),
      browser_type.as_str(),
      version
    );

    // Detect the actual file type by reading the file header
    let actual_format = self.detect_file_format(archive_path).map_err(|e| {
      format!(
        "Failed to detect file format for {}: {}",
        archive_path.display(),
        e
      )
    })?;
    log::info!("Detected format: {actual_format}");

    let extraction_result = match actual_format.as_str() {
      "dmg" => {
        #[cfg(target_os = "macos")]
        {
          self.extract_dmg(archive_path, dest_dir).await.map_err(|e| {
            format!("DMG extraction failed for {} {}: {}", browser_type.as_str(), version, e).into()
          })
        }

        #[cfg(not(target_os = "macos"))]
        {
          Err(format!("DMG extraction is only supported on macOS, but {} {} requires DMG extraction", browser_type.as_str(), version).into())
        }
      }
      "zip" => {
        self.extract_zip(archive_path, dest_dir).await.map_err(|e| {
          format!("ZIP extraction failed for {} {}: {}", browser_type.as_str(), version, e).into()
        })
      }
      "tar.xz" => {
        self.extract_tar_xz(archive_path, dest_dir).await.map_err(|e| {
          format!("TAR.XZ extraction failed for {} {}: {}", browser_type.as_str(), version, e).into()
        })
      }
      "tar.bz2" => {
        self.extract_tar_bz2(archive_path, dest_dir).await.map_err(|e| {
          format!("TAR.BZ2 extraction failed for {} {}: {}", browser_type.as_str(), version, e).into()
        })
      }
      "tar.gz" => {
        self.extract_tar_gz(archive_path, dest_dir).await.map_err(|e| {
          format!("TAR.GZ extraction failed for {} {}: {}", browser_type.as_str(), version, e).into()
        })
      }
      "msi" => {
        self.extract_msi(archive_path, dest_dir).await.map_err(|e| {
          format!("MSI extraction failed for {} {}: {}", browser_type.as_str(), version, e).into()
        })
      }
      "exe" => {
        // For Windows EXE files, some may be self-extracting archives, others are installers
        // For browsers like Firefox, TOR, they're typically installers that don't need extraction
        self
          .handle_exe_file(archive_path, dest_dir, browser_type.clone())
          .await
          .map_err(|e| {
            format!("EXE handling failed for {} {}: {}", browser_type.as_str(), version, e).into()
          })
      }
      "appimage" => {
        #[cfg(target_os = "linux")]
        {
          self.handle_appimage(archive_path, dest_dir).await.map_err(|e| {
            format!("AppImage handling failed for {} {}: {}", browser_type.as_str(), version, e).into()
          })
        }

        #[cfg(not(target_os = "linux"))]
        {
          Err(format!("AppImage is only supported on Linux, but {} {} requires AppImage handling", browser_type.as_str(), version).into())
        }
      }
      _ => {
        Err(format!(
          "Unsupported archive format for {} {}: {} (detected: {}). The downloaded file might be corrupted or in an unexpected format. File: {}",
          browser_type.as_str(),
          version,
          archive_path.extension().and_then(|ext| ext.to_str()).unwrap_or("unknown"),
          actual_format,
          archive_path.display()
        ).into())
      }
    };

    match extraction_result {
      Ok(path) => {
        log::info!(
          "Successfully extracted {} {} to: {}",
          browser_type.as_str(),
          version,
          path.display()
        );
        Ok(path)
      }
      Err(e) => {
        log::error!(
          "Extraction failed for {} {}: {}",
          browser_type.as_str(),
          version,
          e
        );
        Err(e)
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
    log::info!(
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

    log::info!("Created mount point: {}", mount_point.display());

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
      log::info!("Failed to mount DMG. stdout: {stdout}, stderr: {stderr}");

      // Clean up mount point before returning error
      let _ = fs::remove_dir_all(&mount_point);

      return Err(format!("Failed to mount DMG: {stderr}").into());
    }

    log::info!("Successfully mounted DMG");

    // Find the .app directory in the mount point
    let app_result = self.find_app_in_directory(&mount_point).await;

    let app_entry = match app_result {
      Ok(app_path) => app_path,
      Err(e) => {
        log::info!("Failed to find .app in mount point: {e}");

        // Try to unmount before returning error
        let _ = Command::new("hdiutil")
          .args(["detach", "-force", mount_point.to_str().unwrap()])
          .output();
        let _ = fs::remove_dir_all(&mount_point);

        return Err("No .app found after extraction".into());
      }
    };

    log::info!("Found .app bundle: {}", app_entry.display());

    // Copy the .app to the destination
    let app_path = dest_dir.join(app_entry.file_name().unwrap());

    log::info!("Copying .app to: {}", app_path.display());

    let output = Command::new("cp")
      .args([
        "-R",
        app_entry.to_str().unwrap(),
        app_path.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      log::info!("Failed to copy app: {stderr}");

      // Unmount before returning error
      let _ = Command::new("hdiutil")
        .args(["detach", "-force", mount_point.to_str().unwrap()])
        .output();
      let _ = fs::remove_dir_all(&mount_point);

      return Err(format!("Failed to copy app: {stderr}").into());
    }

    log::info!("Successfully copied .app bundle");

    // Remove quarantine attributes
    let _ = Command::new("xattr")
      .args(["-dr", "com.apple.quarantine", app_path.to_str().unwrap()])
      .output();

    let _ = Command::new("xattr")
      .args(["-cr", app_path.to_str().unwrap()])
      .output();

    log::info!("Removed quarantine attributes");

    // Unmount the DMG
    let output = Command::new("hdiutil")
      .args(["detach", mount_point.to_str().unwrap()])
      .output()?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      log::warn!("Warning: Failed to unmount DMG: {stderr}");
      // Don't fail if unmount fails - the extraction was successful
    } else {
      log::info!("Successfully unmounted DMG");
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
              log::info!("Found .app bundle at depth {}: {}", depth, path.display());
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
    log::info!("Extracting ZIP archive: {}", zip_path.display());
    std::fs::create_dir_all(dest_dir)?;

    let file = File::open(zip_path)
      .map_err(|e| format!("Failed to open ZIP file {}: {}", zip_path.display(), e))?;

    let mut archive = zip::ZipArchive::new(BufReader::new(file))
      .map_err(|e| format!("Failed to read ZIP archive {}: {}", zip_path.display(), e))?;

    log::info!("ZIP archive contains {} files", archive.len());

    for i in 0..archive.len() {
      let mut entry = archive
        .by_index(i)
        .map_err(|e| format!("Failed to read ZIP entry at index {i}: {e}"))?;

      // Use enclosed_name to prevent path traversal; do not modify names otherwise
      let enclosed = entry
        .enclosed_name()
        .ok_or_else(|| format!("ZIP contains an invalid entry path: {}", entry.name()))?;

      let outpath = dest_dir.join(enclosed);

      // Handle directories and files
      if entry.is_dir() {
        std::fs::create_dir_all(&outpath)
          .map_err(|e| format!("Failed to create directory {}: {}", outpath.display(), e))?;
      } else {
        if let Some(parent) = outpath.parent() {
          std::fs::create_dir_all(parent).map_err(|e| {
            format!(
              "Failed to create parent directory {}: {}",
              parent.display(),
              e
            )
          })?;
        }

        let mut outfile = File::create(&outpath)
          .map_err(|e| format!("Failed to create file {}: {}", outpath.display(), e))?;
        io::copy(&mut entry, &mut outfile)
          .map_err(|e| format!("Failed to extract file {}: {}", outpath.display(), e))?;

        // Set executable permissions on Unix-like systems based on stored mode
        #[cfg(unix)]
        {
          use std::os::unix::fs::PermissionsExt;
          if let Some(mode) = entry.unix_mode() {
            let permissions = std::fs::Permissions::from_mode(mode);
            std::fs::set_permissions(&outpath, permissions)
              .map_err(|e| format!("Failed to set permissions for {}: {}", outpath.display(), e))?;
          }
        }
      }
    }

    log::info!("ZIP extraction completed.");

    self.flatten_single_directory_archive(dest_dir)?;

    log::info!("Searching for executable...");
    self
      .find_extracted_executable(dest_dir)
      .await
      .map_err(|e| format!("Failed to find executable after ZIP extraction: {e}").into())
  }

  pub async fn extract_tar_gz(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Extracting tar.gz archive: {}", tar_path.display());
    std::fs::create_dir_all(dest_dir)?;

    let file = File::open(tar_path)?;
    let gz_decoder = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(gz_decoder);

    archive.unpack(dest_dir)?;

    // Set executable permissions for extracted files
    self.set_executable_permissions_recursive(dest_dir).await?;

    log::info!("tar.gz extraction completed.");
    self.flatten_single_directory_archive(dest_dir)?;
    log::info!("Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_tar_bz2(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Extracting tar.bz2 archive: {}", tar_path.display());
    std::fs::create_dir_all(dest_dir)?;

    let file = File::open(tar_path)?;
    let bz2_decoder = bzip2::read::BzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(bz2_decoder);

    archive.unpack(dest_dir)?;

    // Set executable permissions for extracted files
    self.set_executable_permissions_recursive(dest_dir).await?;

    log::info!("tar.bz2 extraction completed.");
    self.flatten_single_directory_archive(dest_dir)?;
    log::info!("Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_tar_xz(
    &self,
    tar_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Extracting tar.xz archive: {}", tar_path.display());
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

    log::info!("tar.xz extraction completed.");
    self.flatten_single_directory_archive(dest_dir)?;
    log::info!("Searching for executable...");
    self.find_extracted_executable(dest_dir).await
  }

  pub async fn extract_msi(
    &self,
    msi_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Extracting MSI archive: {}", msi_path.display());
    std::fs::create_dir_all(dest_dir)?;

    // Extract MSI in a separate scope to avoid Send issues
    {
      let mut extractor = msi_extract::MsiExtractor::from_path(msi_path)?;
      extractor.to(dest_dir);
    }

    log::info!("MSI extraction completed.");
    self.flatten_single_directory_archive(dest_dir)?;
    log::info!("Searching for executable...");
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
    self
      .set_executable_permissions_recursive(&dest_file)
      .await?;

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

  fn flatten_single_directory_archive(
    &self,
    dest_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let entries: Vec<_> = fs::read_dir(dest_dir)?.filter_map(|e| e.ok()).collect();

    let archive_extensions = ["zip", "tar", "xz", "gz", "bz2", "dmg", "msi", "exe"];

    let mut dirs = Vec::new();
    let mut has_non_archive_files = false;

    for entry in &entries {
      let path = entry.path();
      if path.is_dir() {
        dirs.push(path);
      } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if !archive_extensions.contains(&ext.to_lowercase().as_str()) {
          has_non_archive_files = true;
        }
      } else {
        has_non_archive_files = true;
      }
    }

    if dirs.len() == 1 && !has_non_archive_files {
      let single_dir = &dirs[0];
      log::info!(
        "Flattening single-directory archive: moving contents of {} to {}",
        single_dir.display(),
        dest_dir.display()
      );

      let inner_entries: Vec<_> = fs::read_dir(single_dir)?.filter_map(|e| e.ok()).collect();

      for entry in inner_entries {
        let source = entry.path();
        let file_name = match source.file_name() {
          Some(name) => name.to_owned(),
          None => continue,
        };
        let target = dest_dir.join(&file_name);
        fs::rename(&source, &target).map_err(|e| {
          format!(
            "Failed to move {} to {}: {}",
            source.display(),
            target.display(),
            e
          )
        })?;
      }

      fs::remove_dir(single_dir).map_err(|e| {
        format!(
          "Failed to remove empty directory {}: {}",
          single_dir.display(),
          e
        )
      })?;

      log::info!("Successfully flattened archive directory structure");
    }

    Ok(())
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
      let result = self.find_linux_executable(dest_dir).await;

      // If we found an executable, ensure it's in the correct directory structure
      // that the verification expects
      if let Ok(exe_path) = &result {
        self
          .ensure_correct_directory_structure(dest_dir, exe_path)
          .await?;
      }

      result
    }
  }

  #[cfg(target_os = "macos")]
  async fn find_macos_app(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Searching for .app bundle in: {}", dest_dir.display());

    // Use the enhanced recursive search
    match self.find_app_in_directory(dest_dir).await {
      Ok(app_path) => {
        // Check if the app is in a subdirectory and move it to the root if needed
        let app_parent = app_path.parent().unwrap();
        if app_parent != dest_dir {
          log::info!(
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

          log::info!("Successfully moved .app to: {}", target_path.display());
          Ok(target_path)
        } else {
          log::info!("Found .app at root level: {}", app_path.display());
          Ok(app_path)
        }
      }
      Err(e) => {
        log::info!("Failed to find .app bundle: {e}");
        Err("No .app found after extraction".into())
      }
    }
  }

  #[cfg(target_os = "windows")]
  async fn find_windows_executable(
    &self,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    log::info!(
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
      "camoufox.exe",
      "wayfern.exe",
    ];

    // First try priority executable names
    for exe_name in &priority_exe_names {
      let exe_path = dest_dir.join(exe_name);
      if exe_path.exists() {
        log::info!("Found priority executable: {}", exe_path.display());
        return Ok(exe_path);
      }
    }

    // Recursively search for executables with depth limit
    match self.find_windows_executable_recursive(dest_dir, 0, 3).await {
      Ok(exe_path) => {
        log::info!(
          "Found executable via recursive search: {}",
          exe_path.display()
        );
        Ok(exe_path)
      }
      Err(_) => Err("No executable found after extraction".into()),
    }
  }

  #[cfg(target_os = "windows")]
  #[allow(clippy::type_complexity)]
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
              || file_name.contains("browser")
              || file_name.contains("camoufox")
              || file_name.contains("wayfern")
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
    log::info!("Searching for Linux executable in: {}", dest_dir.display());

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
      // Camoufox variants
      "camoufox",
      "camoufox-bin",
      "camoufox-browser",
      // Wayfern variants
      "wayfern",
      "wayfern-bin",
      "wayfern-browser",
    ];

    // First, try direct lookup in the main directory
    for exe_name in &exe_names {
      let exe_path = dest_dir.join(exe_name);
      if exe_path.exists() && self.is_executable(&exe_path) {
        log::info!("Found executable at root level: {}", exe_path.display());
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
      "camoufox",
      "wayfern",
      ".",
      "./",
      "firefox",
      "Browser",
      "browser",
      "opt/google/chrome",
      "opt/brave.com/brave",
      "opt/camoufox",
      "usr/lib/firefox",
      "usr/lib/chromium",
      "usr/lib/camoufox",
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
            log::info!("Found executable in subdirectory: {}", exe_path.display());
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
            log::info!("Found AppImage: {}", path.display());
            return Ok(path);
          }
        }
      }
    }

    // Last resort: recursive search for any executable file
    log::info!("Performing recursive search for executables...");
    match self.find_any_executable_recursive(dest_dir, 0).await {
      Ok(path) => {
        log::info!("Found executable via recursive search: {}", path.display());
        Ok(path)
      }
      Err(e) => {
        // List all files in the directory for debugging
        log::info!("Failed to find executable. Directory contents:");
        if let Ok(entries) = fs::read_dir(dest_dir) {
          for entry in entries.flatten() {
            let path = entry.path();
            let is_exec = if path.is_file() {
              self.is_executable(&path)
            } else {
              false
            };
            log::info!("  {} (executable: {})", path.display(), is_exec);
          }
        }
        Err(
          format!(
            "No executable found in {} after extraction. Original error: {}",
            dest_dir.display(),
            e
          )
          .into(),
        )
      }
    }
  }

  #[cfg(target_os = "linux")]
  fn is_executable(&self, path: &Path) -> bool {
    if let Ok(metadata) = path.metadata() {
      use std::os::unix::fs::PermissionsExt;
      return metadata.permissions().mode() & 0o111 != 0;
    }
    false
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
              || name_lower.contains("camoufox")
              || name_lower.contains("wayfern")
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
      let mut potential_executables = Vec::new();

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
              || name_lower.contains("camoufox")
              || name_lower.contains("wayfern")
              || file_name.ends_with(".AppImage")
            {
              log::info!(
                "Found priority executable at depth {}: {}",
                depth,
                path.display()
              );
              return Ok(path);
            }
            // Collect other executables as potential candidates
            potential_executables.push(path);
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

      // Third pass: if no browser-specific executable found, try any executable
      if !potential_executables.is_empty() {
        // Sort by filename to prefer more likely candidates
        potential_executables.sort_by(|a, b| {
          let a_name = a
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
          let b_name = b
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();

          // Prefer shorter names (likely main executables)
          a_name.len().cmp(&b_name.len())
        });

        log::info!(
          "Found potential executable at depth {}: {}",
          depth,
          potential_executables[0].display()
        );
        return Ok(potential_executables[0].clone());
      }
    }

    Err(format!("No executable found in directory: {}", dir.display()).into())
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
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let dest_dir = temp_dir.path().join("extracted");

    // Create a test ZIP archive in memory
    let zip_path = temp_dir.path().join("test.zip");
    {
      let file = std::fs::File::create(&zip_path).expect("Failed to create test zip file");
      let mut zip = zip::ZipWriter::new(file);

      let options =
        zip::write::FileOptions::<()>::default().compression_method(zip::CompressionMethod::Stored);

      zip
        .start_file("test.txt", options)
        .expect("Failed to start zip file");
      zip
        .write_all(b"Hello, World!")
        .expect("Failed to write to zip");
      zip.finish().expect("Failed to finish zip");
    }

    let result = extractor.extract_zip(&zip_path, &dest_dir).await;

    // The result might fail because we're looking for executables, but the extraction should work
    // Let's check if the file was extracted regardless of the result
    let extracted_file = dest_dir.join("test.txt");
    assert!(extracted_file.exists(), "Extracted file should exist");

    let content = std::fs::read_to_string(&extracted_file).expect("Failed to read extracted file");
    assert_eq!(
      content.trim(),
      "Hello, World!",
      "Extracted content should match"
    );

    // If the result is an error, it should be because no executable was found, not extraction failure
    if let Err(e) = result {
      let error_msg = e.to_string();
      assert!(
        error_msg.contains("No executable found") || error_msg.contains("executable"),
        "Error should be about missing executable, not extraction failure: {error_msg}"
      );
    }
  }

  #[tokio::test]
  async fn test_extract_tar_gz_with_test_archive() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let dest_dir = temp_dir.path().join("extracted");

    // Create a test tar.gz archive in memory
    let tar_gz_path = temp_dir.path().join("test.tar.gz");
    {
      let tar_gz_file =
        std::fs::File::create(&tar_gz_path).expect("Failed to create test tar.gz file");
      let enc = flate2::write::GzEncoder::new(tar_gz_file, flate2::Compression::default());
      let mut tar = tar::Builder::new(enc);

      let mut header = tar::Header::new_gnu();
      header.set_path("test.txt").expect("Failed to set tar path");
      header.set_size(13); // "Hello, World!" length
      header.set_cksum();

      tar
        .append(&header, "Hello, World!".as_bytes())
        .expect("Failed to append to tar");
      tar.finish().expect("Failed to finish tar");
    }

    let result = extractor.extract_tar_gz(&tar_gz_path, &dest_dir).await;

    // Check if the file was extracted
    let extracted_file = dest_dir.join("test.txt");
    assert!(extracted_file.exists(), "Extracted file should exist");

    let content = std::fs::read_to_string(&extracted_file).expect("Failed to read extracted file");
    assert_eq!(
      content.trim(),
      "Hello, World!",
      "Extracted content should match"
    );

    // If the result is an error, it should be because no executable was found, not extraction failure
    if let Err(e) = result {
      let error_msg = e.to_string();
      assert!(
        error_msg.contains("No executable found")
          || error_msg.contains("executable")
          || error_msg.contains("No .app found")
          || error_msg.contains("app not found"),
        "Error should be about missing executable/app, not extraction failure: {error_msg}"
      );
    }
  }

  #[tokio::test]
  async fn test_extract_tar_bz2_with_test_archive() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let dest_dir = temp_dir.path().join("extracted");

    // Create a test tar.bz2 archive in memory
    let tar_bz2_path = temp_dir.path().join("test.tar.bz2");
    {
      let tar_bz2_file =
        std::fs::File::create(&tar_bz2_path).expect("Failed to create test tar.bz2 file");
      let enc = bzip2::write::BzEncoder::new(tar_bz2_file, bzip2::Compression::default());
      let mut tar = tar::Builder::new(enc);

      let mut header = tar::Header::new_gnu();
      header.set_path("test.txt").expect("Failed to set tar path");
      header.set_size(13); // "Hello, World!" length
      header.set_cksum();

      tar
        .append(&header, "Hello, World!".as_bytes())
        .expect("Failed to append to tar");
      tar.finish().expect("Failed to finish tar");
    }

    let result = extractor.extract_tar_bz2(&tar_bz2_path, &dest_dir).await;

    // Check if the file was extracted
    let extracted_file = dest_dir.join("test.txt");
    assert!(extracted_file.exists(), "Extracted file should exist");

    let content = std::fs::read_to_string(&extracted_file).expect("Failed to read extracted file");
    assert_eq!(
      content.trim(),
      "Hello, World!",
      "Extracted content should match"
    );

    // If the result is an error, it should be because no executable was found, not extraction failure
    if let Err(e) = result {
      let error_msg = e.to_string();
      assert!(
        error_msg.contains("No executable found")
          || error_msg.contains("executable")
          || error_msg.contains("No .app found")
          || error_msg.contains("app not found"),
        "Error should be about missing executable/app, not extraction failure: {error_msg}"
      );
    }
  }

  #[tokio::test]
  async fn test_extract_tar_xz_with_test_archive() {
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let dest_dir = temp_dir.path().join("extracted");

    // Create a test tar.xz archive in memory
    let tar_xz_path = temp_dir.path().join("test.tar.xz");
    {
      // First create a tar archive in memory
      let mut tar_data = Vec::new();
      {
        let mut tar = tar::Builder::new(&mut tar_data);

        let mut header = tar::Header::new_gnu();
        header.set_path("test.txt").expect("Failed to set tar path");
        header.set_size(13); // "Hello, World!" length
        header.set_cksum();

        tar
          .append(&header, "Hello, World!".as_bytes())
          .expect("Failed to append to tar");
        tar.finish().expect("Failed to finish tar");
      }

      // Then compress with xz
      let tar_xz_file =
        std::fs::File::create(&tar_xz_path).expect("Failed to create test tar.xz file");
      let mut compressed_data = Vec::new();
      lzma_rs::xz_compress(&mut std::io::Cursor::new(tar_data), &mut compressed_data)
        .expect("Failed to compress with xz");
      std::io::Write::write_all(&mut std::io::BufWriter::new(tar_xz_file), &compressed_data)
        .expect("Failed to write compressed data");
    }

    let result = extractor.extract_tar_xz(&tar_xz_path, &dest_dir).await;

    // Check if the file was extracted
    let extracted_file = dest_dir.join("test.txt");
    assert!(extracted_file.exists(), "Extracted file should exist");

    let content = std::fs::read_to_string(&extracted_file).expect("Failed to read extracted file");
    assert_eq!(
      content.trim(),
      "Hello, World!",
      "Extracted content should match"
    );

    // If the result is an error, it should be because no executable was found, not extraction failure
    if let Err(e) = result {
      let error_msg = e.to_string();
      assert!(
        error_msg.contains("No executable found")
          || error_msg.contains("executable")
          || error_msg.contains("No .app found")
          || error_msg.contains("app not found"),
        "Error should be about missing executable/app, not extraction failure: {error_msg}"
      );
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

  #[test]
  fn test_is_executable() {
    #[allow(unused_variables)]
    let extractor = Extractor::instance();
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create a regular file
    let regular_file = temp_dir.path().join("regular.txt");
    File::create(&regular_file).expect("Failed to create test file");

    #[cfg(target_os = "linux")]
    {
      // Should not be executable initially
      assert!(
        !extractor.is_executable(&regular_file),
        "File should not be executable initially"
      );

      // Make it executable
      use std::os::unix::fs::PermissionsExt;
      let mut permissions = regular_file
        .metadata()
        .expect("Failed to get file metadata")
        .permissions();
      permissions.set_mode(0o755);
      std::fs::set_permissions(&regular_file, permissions).expect("Failed to set permissions");

      // Should now be executable
      assert!(
        extractor.is_executable(&regular_file),
        "File should be executable after setting permissions"
      );
    }

    #[cfg(not(target_os = "linux"))]
    {
      // On non-Linux systems, the is_executable method is not available
      // We'll just verify the file exists since executable permissions work differently on Windows/macOS
      assert!(regular_file.exists(), "Test file should exist");

      // On Unix systems (but not Linux), we can still test basic permission setting
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = regular_file
          .metadata()
          .expect("Failed to get file metadata")
          .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&regular_file, permissions).expect("Failed to set permissions");

        // Verify the permissions were set
        let new_permissions = regular_file
          .metadata()
          .expect("Failed to get updated metadata")
          .permissions();
        assert_eq!(
          new_permissions.mode() & 0o777,
          0o755,
          "Permissions should be set to 755"
        );
      }
    }
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref EXTRACTOR: Extractor = Extractor::new();
}
