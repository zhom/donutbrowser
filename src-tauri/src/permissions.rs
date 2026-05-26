#[tauri::command]
pub async fn open_macos_permission_settings(permission_type: String) -> Result<(), String> {
  #[cfg(target_os = "macos")]
  {
    let pane = match permission_type.as_str() {
      "microphone" => "Privacy_Microphone",
      "camera" => "Privacy_Camera",
      other => {
        return Err(
          serde_json::json!({
            "code": "UNSUPPORTED_PERMISSION_TYPE",
            "params": { "type": other },
          })
          .to_string(),
        );
      }
    };

    std::process::Command::new("open")
      .arg(format!(
        "x-apple.systempreferences:com.apple.preference.security?{pane}"
      ))
      .status()
      .map_err(|error| error.to_string())?;
  }

  #[cfg(not(target_os = "macos"))]
  {
    let _ = permission_type;
  }

  Ok(())
}
