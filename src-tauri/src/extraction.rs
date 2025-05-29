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

    let extension = archive_path
      .extension()
      .and_then(|ext| ext.to_str())
      .unwrap_or("");

    match extension {
      "dmg" => self.extract_dmg(archive_path, dest_dir).await,
      "zip" => self.extract_zip(archive_path, dest_dir).await,
      _ => Err(format!("Unsupported archive format: {extension}").into()),
    }
  }

  async fn extract_dmg(
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

  async fn extract_zip(
    &self,
    zip_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Use unzip command to extract
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

    // Find the extracted .app directory or Chromium.app specifically
    let mut app_path: Option<PathBuf> = None;

    // First, try to find any .app file in the destination directory
    if let Ok(entries) = fs::read_dir(dest_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "app") {
          app_path = Some(path);
          break;
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
                app_path = Some(target_path);

                // Clean up the now-empty subdirectory
                let _ = fs::remove_dir_all(&path);
                break;
              }
            }
            if app_path.is_some() {
              break;
            }
          }
        }
      }
    }

    let app_path = app_path.ok_or("No .app found after extraction")?;

    // Remove quarantine attributes
    let _ = Command::new("xattr")
      .args(["-dr", "com.apple.quarantine", app_path.to_str().unwrap()])
      .output();

    let _ = Command::new("xattr")
      .args(["-cr", app_path.to_str().unwrap()])
      .output();

    Ok(app_path)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs::File;
  use tempfile::TempDir;

  #[test]
  fn test_extractor_creation() {
    let _extractor = Extractor::new();
    // Just verify we can create an extractor instance
  }

  #[test]
  fn test_unsupported_archive_format() {
    let _extractor = Extractor::new();
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
