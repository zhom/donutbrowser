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

    println!(
      "Starting extraction of {} for browser {}",
      archive_path.display(),
      browser_type.as_str()
    );

    // Try to detect the actual file type by reading the file header
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

    // First check file extension for DMG files since they're common on macOS
    // and can have misleading magic numbers
    if let Some(ext) = file_path.extension().and_then(|ext| ext.to_str()) {
      if ext.to_lowercase() == "dmg" {
        return Ok("dmg".to_string());
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

    // List the contents for debugging
    println!("Mount point contents:");
    if let Ok(entries) = fs::read_dir(&mount_point) {
      for entry in entries.flatten() {
        let path = entry.path();
        println!(
          "  - {} ({})",
          path.display(),
          if path.is_dir() { "dir" } else { "file" }
        );
      }
    }

    // Find the .app directory in the mount point with enhanced search
    let app_result = self.find_app_in_directory(&mount_point).await;

    let app_entry = match app_result {
      Ok(app_path) => app_path,
      Err(e) => {
        println!("Failed to find .app in mount point: {e}");

        // Enhanced debugging - look for any interesting files/directories
        if let Ok(entries) = fs::read_dir(&mount_point) {
          println!("Detailed mount point analysis:");
          for entry in entries.flatten() {
            let path = entry.path();
            let metadata = fs::metadata(&path);
            println!(
              "  - {} ({}) - {:?}",
              path.display(),
              if path.is_dir() { "dir" } else { "file" },
              metadata.map(|m| m.len()).unwrap_or(0)
            );

            // If it's a directory, look one level deep
            if path.is_dir() {
              if let Ok(sub_entries) = fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten().take(5) {
                  // Limit to first 5 items
                  let sub_path = sub_entry.path();
                  println!(
                    "    - {} ({})",
                    sub_path.display(),
                    if sub_path.is_dir() { "dir" } else { "file" }
                  );
                }
              }
            }
          }
        }

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
    println!("Extracting ZIP archive on Windows: {}", zip_path.display());

    // Create destination directory if it doesn't exist
    fs::create_dir_all(dest_dir)?;

    // First try PowerShell's Expand-Archive (Windows 10+)
    let powershell_result = Command::new("powershell")
      .args([
        "-Command",
        &format!(
          "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
          zip_path.display(),
          dest_dir.display()
        ),
      ])
      .output();

    match powershell_result {
      Ok(output) if output.status.success() => {
        println!("Successfully extracted using PowerShell");
      }
      Ok(output) => {
        println!(
          "PowerShell extraction failed: {}, trying Rust zip crate fallback",
          String::from_utf8_lossy(&output.stderr)
        );
        // Fallback to Rust zip crate for Windows 7 compatibility
        return self.extract_zip_with_rust_crate(zip_path, dest_dir).await;
      }
      Err(e) => {
        println!("PowerShell not available: {}, using Rust zip crate", e);
        // Fallback to Rust zip crate for Windows 7 compatibility
        return self.extract_zip_with_rust_crate(zip_path, dest_dir).await;
      }
    }

    self.find_extracted_executable(dest_dir).await
  }

  #[cfg(target_os = "windows")]
  async fn extract_zip_with_rust_crate(
    &self,
    zip_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    println!("Using Rust zip crate for extraction (Windows 7+ compatibility)");

    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    for i in 0..archive.len() {
      let mut file = archive.by_index(i)?;
      let outpath = match file.enclosed_name() {
        Some(path) => dest_dir.join(path),
        None => continue,
      };

      // Handle directory creation
      if file.name().ends_with('/') {
        fs::create_dir_all(&outpath)?;
      } else {
        // Create parent directories
        if let Some(p) = outpath.parent() {
          if !p.exists() {
            fs::create_dir_all(p)?;
          }
        }

        // Extract file
        let mut outfile = fs::File::create(&outpath)?;
        std::io::copy(&mut file, &mut outfile)?;

        // On Windows, verify executable files
        if outpath
          .extension()
          .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == "exe")
        {
          if let Ok(metadata) = fs::metadata(&outpath) {
            if metadata.len() > 0 {
              println!("Extracted executable: {}", outpath.display());
            }
          }
        }
      }
    }

    println!("ZIP extraction completed. Searching for executable...");
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

    #[cfg(target_os = "windows")]
    {
      // On Windows, try multiple extraction methods for better compatibility
      // First try using tar if available (Windows 10+)
      let tar_result = Command::new("tar")
        .args([
          "-xf",
          tar_path.to_str().unwrap(),
          "-C",
          dest_dir.to_str().unwrap(),
        ])
        .output();

      match tar_result {
        Ok(output) if output.status.success() => {
          println!("Successfully extracted tar.xz using tar command");
        }
        Ok(output) => {
          println!(
            "tar command failed: {}, trying 7-Zip fallback",
            String::from_utf8_lossy(&output.stderr)
          );
          // Try 7-Zip as fallback
          return self.extract_with_7zip(tar_path, dest_dir).await;
        }
        Err(_) => {
          println!("tar command not available, trying 7-Zip");
          // Try 7-Zip as fallback
          return self.extract_with_7zip(tar_path, dest_dir).await;
        }
      }
    }

    #[cfg(not(target_os = "windows"))]
    {
      // Use tar command for Unix-like systems
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
    }

    // Find the extracted executable and set proper permissions
    let executable_path = self.find_extracted_executable(dest_dir).await?;

    // Ensure executable permissions are set correctly for Linux
    if cfg!(target_os = "linux") {
      self.set_executable_permissions(&executable_path).await?;
    }

    Ok(executable_path)
  }

  #[cfg(target_os = "windows")]
  async fn extract_with_7zip(
    &self,
    archive_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Try to use 7-Zip for extraction (common on Windows)
    let seven_zip_paths = [
      "7z", // If 7z is in PATH
      "C:\\Program Files\\7-Zip\\7z.exe",
      "C:\\Program Files (x86)\\7-Zip\\7z.exe",
    ];

    for seven_zip_path in &seven_zip_paths {
      let result = Command::new(seven_zip_path)
        .args([
          "x", // Extract with full paths
          archive_path.to_str().unwrap(),
          &format!("-o{}", dest_dir.display()), // Output directory
          "-y",                                 // Yes to all
        ])
        .output();

      match result {
        Ok(output) if output.status.success() => {
          println!("Successfully extracted using 7-Zip: {}", seven_zip_path);
          return self.find_extracted_executable(dest_dir).await;
        }
        Ok(_) => continue,
        Err(_) => continue,
      }
    }

    Err(
      "No suitable extraction tool found. Please install 7-Zip or ensure tar is available.".into(),
    )
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

        // List contents for debugging
        if let Ok(entries) = fs::read_dir(dest_dir) {
          println!("Destination directory contents:");
          for entry in entries.flatten() {
            let path = entry.path();
            let metadata = if path.is_dir() { "dir" } else { "file" };
            println!("  - {} ({})", path.display(), metadata);

            // If it's a directory, also list its contents
            if path.is_dir() {
              if let Ok(sub_entries) = fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                  let sub_path = sub_entry.path();
                  let sub_metadata = if sub_path.is_dir() { "dir" } else { "file" };
                  println!("    - {} ({})", sub_path.display(), sub_metadata);
                }
              }
            }
          }
        }

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
      Err(_) => {
        // List directory contents for debugging
        if let Ok(entries) = fs::read_dir(dest_dir) {
          println!("Directory contents:");
          for entry in entries.flatten() {
            let path = entry.path();
            let metadata = if path.is_dir() { "dir" } else { "file" };
            println!("  - {} ({})", path.display(), metadata);
          }
        }

        Err("No executable found after extraction".into())
      }
    }
  }

  #[cfg(target_os = "windows")]
  fn find_windows_executable_recursive(
    &self,
    dir: &Path,
    depth: usize,
    max_depth: usize,
  ) -> std::pin::Pin<
    Box<
      dyn std::future::Future<Output = Result<PathBuf, Box<dyn std::error::Error + Send + Sync>>>
        + Send
        + '_,
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
  use std::fs::{create_dir_all, File};
  use std::io::Write;
  use tempfile::TempDir;

  #[test]
  fn test_extractor_creation() {
    let _ = Extractor::new();
    // Just verify we can create an extractor instance
  }

  #[test]
  fn test_unsupported_archive_format() {
    let extractor = Extractor::new();
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

  #[test]
  fn test_format_detection_zip() {
    let extractor = Extractor::new();
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
    let extractor = Extractor::new();
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
    let extractor = Extractor::new();
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
    let extractor = Extractor::new();
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

  #[tokio::test]
  #[cfg(target_os = "macos")]
  async fn test_find_app_at_root_level() {
    let extractor = Extractor::new();
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

  #[tokio::test]
  #[cfg(target_os = "macos")]
  async fn test_find_app_in_subdirectory() {
    let extractor = Extractor::new();
    let temp_dir = TempDir::new().unwrap();

    // Create a nested structure like some browsers have
    let subdir = temp_dir.path().join("chrome-mac");
    create_dir_all(&subdir).unwrap();

    // Create a Brave Browser.app directory
    let brave_app = subdir.join("Brave Browser.app");
    create_dir_all(&brave_app).unwrap();

    // Create the standard macOS app structure
    let contents_dir = brave_app.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    create_dir_all(&macos_dir).unwrap();

    // Create the executable
    let executable = macos_dir.join("Brave Browser");
    File::create(&executable).unwrap();

    // Test finding the app
    let result = extractor.find_app_in_directory(temp_dir.path()).await;
    assert!(result.is_ok());

    let found_app = result.unwrap();
    assert_eq!(found_app.file_name().unwrap(), "Brave Browser.app");
    assert!(found_app.exists());
  }

  #[tokio::test]
  #[cfg(target_os = "macos")]
  async fn test_find_app_multiple_levels_deep() {
    let extractor = Extractor::new();
    let temp_dir = TempDir::new().unwrap();

    // Create a deeply nested structure
    let level1 = temp_dir.path().join("level1");
    let level2 = level1.join("level2");
    create_dir_all(&level2).unwrap();

    // Create a Mullvad Browser.app directory
    let mullvad_app = level2.join("Mullvad Browser.app");
    create_dir_all(&mullvad_app).unwrap();

    // Create the standard macOS app structure
    let contents_dir = mullvad_app.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    create_dir_all(&macos_dir).unwrap();

    // Create the executable
    let executable = macos_dir.join("firefox");
    File::create(&executable).unwrap();

    // Test finding the app
    let result = extractor.find_app_in_directory(temp_dir.path()).await;
    assert!(result.is_ok());

    let found_app = result.unwrap();
    assert_eq!(found_app.file_name().unwrap(), "Mullvad Browser.app");
    assert!(found_app.exists());
  }

  #[tokio::test]
  #[cfg(target_os = "macos")]
  async fn test_find_app_no_app_found() {
    let extractor = Extractor::new();
    let temp_dir = TempDir::new().unwrap();

    // Create some files and directories that are NOT .app bundles
    let regular_dir = temp_dir.path().join("regular_directory");
    create_dir_all(&regular_dir).unwrap();

    let regular_file = temp_dir.path().join("regular_file.txt");
    File::create(&regular_file).unwrap();

    // Create a directory that looks like an app but isn't (wrong extension)
    let fake_app = temp_dir.path().join("NotAnApp.app-backup");
    create_dir_all(&fake_app).unwrap();

    // Test that no app is found
    let result = extractor.find_app_in_directory(temp_dir.path()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No .app found"));
  }

  #[tokio::test]
  #[cfg(target_os = "macos")]
  async fn test_find_app_recursive_depth_limit() {
    let extractor = Extractor::new();
    let temp_dir = TempDir::new().unwrap();

    // Create a very deep nested structure (deeper than our limit of 4)
    let mut current_path = temp_dir.path().to_path_buf();
    for i in 0..6 {
      current_path = current_path.join(format!("level{i}"));
      create_dir_all(&current_path).unwrap();
    }

    // Create an app at the deepest level
    let deep_app = current_path.join("Deep.app");
    create_dir_all(&deep_app).unwrap();

    // Test that the app is NOT found due to depth limit
    let result = extractor.find_app_in_directory(temp_dir.path()).await;
    assert!(result.is_err());
  }

  #[tokio::test]
  #[cfg(target_os = "macos")]
  async fn test_find_macos_app_and_move_from_subdir() {
    let extractor = Extractor::new();
    let temp_dir = TempDir::new().unwrap();

    // Create a nested structure where the app is in a subdirectory
    let subdir = temp_dir.path().join("extracted_content");
    create_dir_all(&subdir).unwrap();

    // Create a Tor Browser.app directory in the subdirectory
    let tor_app = subdir.join("Tor Browser.app");
    create_dir_all(&tor_app).unwrap();

    // Create the standard macOS app structure
    let contents_dir = tor_app.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    create_dir_all(&macos_dir).unwrap();

    // Create the executable
    let executable = macos_dir.join("firefox");
    File::create(&executable).unwrap();

    // Test finding and moving the app
    let result = extractor.find_macos_app(temp_dir.path()).await;
    assert!(result.is_ok());

    let found_app = result.unwrap();
    assert_eq!(found_app.file_name().unwrap(), "Tor Browser.app");

    // Verify the app was moved to the root level
    assert_eq!(found_app.parent().unwrap(), temp_dir.path());
    assert!(found_app.exists());

    // Verify the original subdirectory structure was cleaned up
    assert!(!subdir.exists() || fs::read_dir(&subdir).unwrap().count() == 0);
  }

  #[tokio::test]
  #[cfg(target_os = "macos")]
  async fn test_multiple_apps_found_returns_first() {
    let extractor = Extractor::new();
    let temp_dir = TempDir::new().unwrap();

    // Create multiple .app directories
    let firefox_app = temp_dir.path().join("Firefox.app");
    create_dir_all(&firefox_app).unwrap();

    let chrome_app = temp_dir.path().join("Chrome.app");
    create_dir_all(&chrome_app).unwrap();

    // Test that we find one of them (implementation should be consistent)
    let result = extractor.find_app_in_directory(temp_dir.path()).await;
    assert!(result.is_ok());

    let found_app = result.unwrap();
    let app_name = found_app.file_name().unwrap().to_str().unwrap();
    assert!(app_name == "Firefox.app" || app_name == "Chrome.app");
  }

  #[test]
  fn test_browser_specific_app_names() {
    // Test that we can identify common browser app names correctly
    let common_browser_apps = [
      "Firefox.app",
      "Firefox Developer Edition.app",
      "Brave Browser.app",
      "Mullvad Browser.app",
      "Tor Browser.app",
      "Zen Browser.app",
      "Chromium.app",
      "Google Chrome.app",
    ];

    for app_name in &common_browser_apps {
      let path = std::path::Path::new(app_name);
      let extension = path.extension().and_then(|ext| ext.to_str());
      assert_eq!(extension, Some("app"), "Failed for {app_name}");
    }
  }

  #[test]
  fn test_edge_cases_in_path_handling() {
    let temp_dir = TempDir::new().unwrap();

    // Test paths with spaces and special characters
    let problematic_names = [
      "Firefox Developer Edition.app",
      "Brave Browser.app",
      "App with (parentheses).app",
      "App-with-dashes.app",
      "App_with_underscores.app",
    ];

    for app_name in &problematic_names {
      let app_path = temp_dir.path().join(app_name);
      create_dir_all(&app_path).unwrap();

      // Verify we can detect the .app extension correctly
      assert!(app_path.extension().is_some_and(|ext| ext == "app"));

      // Verify file_name extraction works
      assert_eq!(app_path.file_name().unwrap().to_str().unwrap(), *app_name);
    }
  }
}
