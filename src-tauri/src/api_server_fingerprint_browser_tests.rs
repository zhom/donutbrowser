use super::*;
use crate::profile::BrowserProfile;
use crate::wayfern_manager::{HeadlessWayfern, WayfernConfig, WayfernManager};
use serde_json::{json, Value};
use std::path::{Path as FsPath, PathBuf};

const LOCATION: &str =
  r#"{"timezone":"Europe/Berlin","language":"de-DE","latitude":52.52,"longitude":13.405}"#;

fn wayfern_dir() -> PathBuf {
  std::env::var("DONUT_TEST_WAYFERN_DIR")
    .ok()
    .filter(|dir| !dir.trim().is_empty())
    .map(PathBuf::from)
    .expect("DONUT_TEST_WAYFERN_DIR is required")
}

/// `DONUT_TEST_WAYFERN_VERSION`, or what the build reports. A Windows build
/// prints no version, so it needs the variable.
fn wayfern_version(dir: &FsPath) -> String {
  if let Some(version) = std::env::var("DONUT_TEST_WAYFERN_VERSION")
    .ok()
    .filter(|version| !version.trim().is_empty())
  {
    return version;
  }
  let exe = crate::browser::create_browser(crate::browser::BrowserType::Wayfern)
    .get_executable_path(dir)
    .expect("DONUT_TEST_WAYFERN_DIR holds a Wayfern build");
  let out = std::process::Command::new(exe)
    .arg("--version")
    .output()
    .expect("the browser reports its version");
  String::from_utf8_lossy(&out.stdout)
    .split_whitespace()
    .find(|word| word.contains('.'))
    .expect("a dotted version, or DONUT_TEST_WAYFERN_VERSION")
    .to_string()
}

/// The data dir's binaries entry for `version`, pointing at the build.
fn install(root: &FsPath, dir: &FsPath, version: &str) {
  let parent = root.join("binaries").join("wayfern");
  std::fs::create_dir_all(&parent).unwrap();
  #[cfg(unix)]
  std::os::unix::fs::symlink(dir, parent.join(version)).unwrap();
  #[cfg(windows)]
  assert!(std::process::Command::new("cmd")
    .args(["/c", "mklink", "/J"])
    .arg(parent.join(version))
    .arg(dir)
    .output()
    .unwrap()
    .status
    .success());
}

fn profile(version: &str, config: WayfernConfig) -> BrowserProfile {
  let profile = BrowserProfile {
    id: uuid::Uuid::new_v4(),
    name: format!("fingerprint {}", uuid::Uuid::new_v4()),
    browser: "wayfern".to_string(),
    version: version.to_string(),
    release_type: "stable".to_string(),
    wayfern_config: Some(config),
    host_os: Some(crate::profile::types::get_host_os()),
    ..Default::default()
  };
  ProfileManager::instance().save_profile(&profile).unwrap();
  profile
}

/// A device minted the way creation and regeneration mint one.
async fn mint(version: &str, os: &str, method: &str, params: Value) -> Value {
  let stand_in = BrowserProfile {
    browser: "wayfern".to_string(),
    version: version.to_string(),
    ..Default::default()
  };
  let token = crate::cloud_auth::CLOUD_AUTH.get_wayfern_token().await;
  let session = HeadlessWayfern::start(
    WayfernManager::instance(),
    &stand_in,
    token.as_deref(),
    "purpose=test",
  )
  .await
  .expect("the browser starts");
  let mut params = params;
  params["operatingSystem"] = json!(os);
  if let Some(token) = token {
    params["wayfernToken"] = json!(token);
  }
  let answer = session
    .call(WayfernManager::instance(), method, params)
    .await;
  session.stop().await;
  answer.unwrap_or_else(|e| panic!("{method} failed: {e}"))
}

async fn minted_identity(version: &str, os: &str) -> String {
  mint(version, os, "Wayfern.createIdentity", json!({})).await["identityId"]
    .as_str()
    .expect("an identity id")
    .to_string()
}

/// What each read path answers for `profile`: the fingerprint form's command,
/// REST and MCP. All three must agree.
async fn read(profile: &BrowserProfile) -> Result<Option<Value>, String> {
  let id = profile.id.to_string();
  let form = crate::get_profile_fingerprint(id.clone()).await;
  let rest = get_profile_fingerprint_api(Path(id.clone()))
    .await
    .map(|Json(answer)| answer.fingerprint)
    .map_err(|(status, body)| format!("{status} {body}"));
  let call = json!({
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {"name": "get_profile_fingerprint", "arguments": {"profile_id": id}}
  });
  let crate::mcp_server::McpOutcome::Body { body, .. } = crate::mcp_server::McpServer::instance()
    .handle_message_with(None, call.to_string().as_bytes(), None)
    .await
  else {
    panic!("MCP did not answer");
  };
  let mcp = match body["result"]["content"][0]["text"].as_str() {
    Some(text) => {
      let info: Value = serde_json::from_str(text).unwrap();
      Ok(info["fingerprint"].as_str().map(str::to_string))
    }
    None => Err(body["error"]["message"].to_string()),
  };

  let parse = |json: Option<String>| json.map(|json| serde_json::from_str::<Value>(&json).unwrap());
  match (form, rest, mcp) {
    (Ok(form), Ok(rest), Ok(mcp)) => {
      let form = parse(form);
      assert_eq!(parse(rest), form, "REST reads what the form reads");
      assert_eq!(parse(mcp), form, "MCP reads what the form reads");
      Ok(form)
    }
    (Err(form), Err(_), Err(_)) => Err(form),
    (form, rest, mcp) => {
      let brief = |answer: &Result<Option<String>, String>| match answer {
        Ok(Some(json)) => format!("{} bytes", json.len()),
        Ok(None) => "none".to_string(),
        Err(e) => format!("error {e}"),
      };
      panic!(
        "the read paths disagree: form {}, REST {}, MCP {}",
        brief(&form),
        brief(&rest),
        brief(&mcp)
      )
    }
  }
}

fn assert_full(device: &Value, what: &str) {
  let fields = device.as_object().map(|o| o.len()).unwrap_or_default();
  println!("{what}: {fields} fields");
  assert!(fields >= 10, "{what}: {fields} fields {device}");
}

/// Reads the device of identity-backed profiles through every path, against a
/// real browser build. Needs an e2e build (`--features e2e`),
/// `TAURI_AUTOMATION=1` and an entitled `WAYFERN_TEST_TOKEN`; the token-less
/// half of the test removes the token from the environment.
#[tokio::test(flavor = "current_thread")]
#[ignore = "needs a browser build in DONUT_TEST_WAYFERN_DIR and WAYFERN_TEST_TOKEN"]
async fn every_read_path_shows_the_device_an_identity_presents() {
  assert!(
    crate::e2e_automation_enabled(),
    "TAURI_AUTOMATION=1 is required"
  );
  assert!(
    std::env::var("WAYFERN_TEST_TOKEN").is_ok_and(|token| !token.is_empty()),
    "WAYFERN_TEST_TOKEN is required"
  );
  crate::mcp_server::McpServer::instance().mark_engine_ready_for_tests();
  let root = tempfile::tempdir().unwrap();
  std::env::set_var("DONUTBROWSER_DATA_DIR", root.path());
  std::env::set_var("DONUTBROWSER_CACHE_DIR", root.path().join("cache"));
  let _data = crate::app_dirs::set_test_data_dir(root.path().to_path_buf());
  let _cache = crate::app_dirs::set_test_cache_dir(root.path().join("cache"));
  let dir = wayfern_dir();
  let version = wayfern_version(&dir);
  assert!(crate::wayfern_manager::supports_wayfern_152(&version));
  install(root.path(), &dir, &version);
  let host = crate::profile::types::get_host_os();
  let other = if host == "windows" {
    "macos"
  } else {
    "windows"
  };

  // A new profile stores its identity and location, never the device.
  let created = profile(
    &version,
    WayfernConfig {
      identity_id: Some(minted_identity(&version, &host).await),
      location: Some(LOCATION.to_string()),
      ..Default::default()
    },
  );
  let device = read(&created)
    .await
    .unwrap()
    .expect("a new profile's device");
  assert_full(&device, "new profile");
  assert_eq!(device["timezone"], json!("Europe/Berlin"));
  assert_eq!(device["language"], json!("de-DE"));
  assert_eq!(
    read(&created).await.unwrap().unwrap(),
    device,
    "the same identity reads as the same device"
  );

  // The user's edits are part of the device the profile presents.
  let edited_cores = if device["hardwareConcurrency"] == json!(4) {
    8
  } else {
    4
  };
  let mut edited = created.clone();
  edited.wayfern_config.as_mut().unwrap().identity_overrides =
    Some(json!({ "hardwareConcurrency": edited_cores }).to_string());
  ProfileManager::instance().save_profile(&edited).unwrap();
  let with_edit = read(&edited).await.unwrap().unwrap();
  assert_eq!(with_edit["hardwareConcurrency"], json!(edited_cores));
  for key in [
    "userAgent",
    "platform",
    "screenWidth",
    "screenHeight",
    "webglRenderer",
  ] {
    assert_eq!(with_edit[key], device[key], "{key} stays the identity's");
  }

  // A claim off this machine's OS.
  let cross_os = profile(
    &version,
    WayfernConfig {
      identity_id: Some(minted_identity(&version, other).await),
      os: Some(other.to_string()),
      location: Some(LOCATION.to_string()),
      ..Default::default()
    },
  );
  let foreign = read(&cross_os).await.unwrap().expect("a cross-OS device");
  assert_full(&foreign, "cross-OS profile");
  assert_eq!(
    foreign["platform"],
    json!(if other == "windows" {
      "Win32"
    } else {
      "MacIntel"
    })
  );

  // A regenerated device replaces the identity; the new one reads in full.
  let mut regenerated = created.clone();
  regenerated.wayfern_config.as_mut().unwrap().identity_id =
    Some(minted_identity(&version, &host).await);
  ProfileManager::instance()
    .save_profile(&regenerated)
    .unwrap();
  let fresh = read(&regenerated).await.unwrap().unwrap();
  assert_full(&fresh, "regenerated profile");
  assert_ne!(fresh, device, "a new identity is a new device");

  // A legacy profile that stored a whole device converts to an identity and
  // still shows that device.
  let payload = mint(&version, &host, "Wayfern.createIdentity", json!({})).await;
  let payload = payload["identity"].clone();
  assert_full(&payload, "legacy payload");
  let legacy = profile(
    &version,
    WayfernConfig {
      fingerprint: Some(payload.to_string()),
      ..Default::default()
    },
  );
  assert_eq!(
    read(&legacy).await.unwrap().unwrap(),
    payload,
    "a stored device is read as stored"
  );
  let migrated = crate::wayfern_identity_storage::convert_for_launch(&legacy)
    .await
    .unwrap();
  let config = migrated.wayfern_config.clone().unwrap();
  assert!(config.identity_id.is_some() && config.fingerprint.is_none());
  let carried = read(&migrated).await.unwrap().expect("a migrated device");
  assert_full(&carried, "migrated profile");
  for key in [
    "userAgent",
    "platform",
    "hardwareConcurrency",
    "screenWidth",
    "screenHeight",
    "webglRenderer",
  ] {
    assert_eq!(
      carried[key], payload[key],
      "{key} carried through migration"
    );
  }

  // Without a token the browser returns no device, so there is none to show.
  std::env::remove_var("WAYFERN_TEST_TOKEN");
  assert_eq!(read(&created).await.unwrap(), None);
  assert_eq!(read(&edited).await.unwrap(), None);
  assert!(
    read(&cross_os).await.is_err(),
    "a cross-OS device is not rebuilt without a plan"
  );
}
