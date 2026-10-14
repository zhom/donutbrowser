use super::*;
use crate::cookie_manager::{chrome_decrypt, CookieManager};
use crate::profile::BrowserProfile;
use crate::wayfern_manager::WayfernManager;
use serde_json::{json, Value};
use std::time::Duration;

const PASSWORD: &str = "correct horse battery";

fn wayfern_executable() -> PathBuf {
  let dir = std::env::var("DONUT_TEST_WAYFERN_DIR")
    .ok()
    .filter(|dir| !dir.trim().is_empty())
    .expect("DONUT_TEST_WAYFERN_DIR is required");
  crate::browser::create_browser(crate::browser::BrowserType::Wayfern)
    .get_executable_path(Path::new(&dir))
    .expect("DONUT_TEST_WAYFERN_DIR holds a Wayfern build")
}

/// Every encrypted cookie row of the profile, and how many of them the
/// profile's `os_crypt_key` opens.
fn decryptable_cookies(user_data_dir: &Path) -> (usize, usize) {
  let path = CookieManager::wayfern_cookie_path(user_data_dir);
  if !path.exists() {
    return (0, 0);
  }
  let conn =
    rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
      .unwrap();
  let mut stmt = conn
    .prepare("SELECT host_key, encrypted_value FROM cookies WHERE length(encrypted_value) > 0")
    .unwrap();
  let rows: Vec<(String, Vec<u8>)> = stmt
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
    .unwrap()
    .map(Result::unwrap)
    .collect();
  let Some(key) = chrome_decrypt::get_encryption_key(user_data_dir) else {
    return (rows.len(), 0);
  };
  let opened = rows
    .iter()
    .filter(|(host, value)| chrome_decrypt::decrypt(value, host, &key).is_some())
    .count();
  (rows.len(), opened)
}

async fn browser_ws_url(port: u16) -> String {
  let client = reqwest::Client::new();
  for _ in 0..150 {
    if let Ok(response) = client
      .get(format!("http://127.0.0.1:{port}/json/version"))
      .send()
      .await
    {
      if let Ok(version) = response.json::<Value>().await {
        if let Some(url) = version["webSocketDebuggerUrl"].as_str() {
          return url.to_string();
        }
      }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
  }
  panic!("the browser never opened its debugging port");
}

async fn cdp(ws_url: &str, method: &str, params: Value) -> Value {
  WayfernManager::instance()
    .send_cdp_command(ws_url, method, params)
    .await
    .unwrap_or_else(|e| panic!("{method} failed: {e}"))
}

/// Runs the browser on `user_data_dir` the way a launch does, checks the
/// cookies earlier sessions set, sets one more and stops it the way the Stop
/// button does. Returns the names of the cookies the browser had on start.
async fn browser_session(user_data_dir: &Path, new_cookie: &str) -> Vec<String> {
  let port = WayfernManager::find_free_port().await.unwrap();
  let mut command = tokio::process::Command::new(wayfern_executable());
  command
    .arg(format!("--remote-debugging-port={port}"))
    .arg("--remote-debugging-address=127.0.0.1")
    .arg(format!("--user-data-dir={}", user_data_dir.display()))
    .arg("--no-first-run")
    .arg("--no-default-browser-check")
    .arg("--disable-background-mode")
    .arg("--disable-component-update")
    .arg("--use-mock-keychain")
    .arg("--password-store=basic")
    .arg("--headless=new")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
  // As a launch does on Linux, where the browser aborts without them.
  #[cfg(target_os = "linux")]
  command.args([
    "--no-sandbox",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage",
  ]);
  let mut child = command.spawn().expect("the browser starts");
  let pid = child.id().unwrap();
  // Reaped as soon as it exits, or the stop below sees a zombie as running.
  let reaper = tokio::spawn(async move { child.wait().await });

  let ws = browser_ws_url(port).await;
  let cookies = cdp(&ws, "Storage.getCookies", json!({})).await;
  let names: Vec<String> = cookies["cookies"]
    .as_array()
    .unwrap()
    .iter()
    .filter(|c| {
      c["domain"]
        .as_str()
        .is_some_and(|d| d.trim_start_matches('.') == "example.com")
    })
    .map(|c| c["name"].as_str().unwrap().to_string())
    .collect();

  let expires = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap()
    .as_secs()
    + 86_400;
  cdp(
    &ws,
    "Storage.setCookies",
    json!({ "cookies": [{
      "name": new_cookie,
      "value": format!("{new_cookie}-value"),
      "domain": "example.com",
      "path": "/",
      "secure": true,
      "expires": expires,
    }]}),
  )
  .await;

  // The cookie store commits every 30 s or at a clean shutdown, and a browser
  // asked to close in its first seconds does not shut down.
  tokio::time::sleep(Duration::from_secs(32)).await;

  WayfernManager::instance()
    .stop_browser_process(pid, Some(port))
    .await;
  let _ = tokio::time::timeout(Duration::from_secs(5), reaper).await;
  names
}

/// The whole life of a password-protected Wayfern profile, several times over:
/// unlock, decrypt, browse, stop, re-encrypt. Each quit is sealed by two
/// callers at once, as the Stop button and the exit watcher do. Every cookie a
/// session set must be readable by the next one.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
#[ignore = "needs a Wayfern build in DONUT_TEST_WAYFERN_DIR"]
async fn cookies_survive_reencryption_through_a_real_browser() {
  let data = tempfile::TempDir::new().unwrap();
  let ephemeral = tempfile::TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(data.path().to_path_buf());
  // Process-wide as well: re-encryption runs on a blocking-pool thread, which
  // the thread-local override above does not reach.
  std::env::set_var("DONUTBROWSER_DATA_DIR", data.path());
  std::env::set_var("DONUTBROWSER_CACHE_DIR", data.path().join("cache"));
  std::env::set_var("DONUTBROWSER_EPHEMERAL_ROOT", ephemeral.path());

  let profile = BrowserProfile {
    id: uuid::Uuid::new_v4(),
    name: "reencrypt".to_string(),
    browser: "wayfern".to_string(),
    release_type: "stable".to_string(),
    ..Default::default()
  };
  ProfileManager::instance().save_profile(&profile).unwrap();
  set_profile_password(profile.id.to_string(), PASSWORD.into())
    .await
    .unwrap();
  let profile = load_profile(&profile.id).unwrap();

  let mut expected: Vec<String> = Vec::new();
  for session in 0..5 {
    if !has_cached_key(&profile.id) {
      unlock_profile(profile.id.to_string(), PASSWORD.into())
        .await
        .unwrap();
    }
    let dir = prepare_for_launch(&profile).unwrap();
    if session == 0 {
      // Bulk like a used profile has, so one seal is still running when the
      // other starts.
      let bulk = dir.join("Default").join("IndexedDB").join("bulk");
      std::fs::create_dir_all(&bulk).unwrap();
      for i in 0..400 {
        std::fs::write(bulk.join(format!("{i:04}.ldb")), vec![7u8; 128 * 1024]).unwrap();
      }
    }
    let (rows, opened) = decryptable_cookies(&dir);
    assert_eq!(
      rows, opened,
      "session {session} found undecryptable cookies"
    );

    let name = format!("c{session}");
    let mut seen = browser_session(&dir, &name).await;
    seen.sort();
    assert_eq!(seen, expected, "session {session} lost cookies");
    expected.push(name);

    let (watcher, stop_button) = tokio::join!(
      complete_after_quit_and_wait(&profile),
      complete_after_quit_and_wait(&profile)
    );
    assert!(
      watcher.is_some() != stop_button.is_some(),
      "session {session} must be sealed exactly once: {watcher:?} {stop_button:?}"
    );
  }

  std::env::remove_var("DONUTBROWSER_EPHEMERAL_ROOT");
  std::env::remove_var("DONUTBROWSER_CACHE_DIR");
  std::env::remove_var("DONUTBROWSER_DATA_DIR");
}
