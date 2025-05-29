use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProxySettings {
  pub enabled: bool,
  pub proxy_type: String, // "http", "https", "socks4", or "socks5"
  pub host: String,
  pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BrowserType {
  MullvadBrowser,
  Chromium,
  Firefox,
  FirefoxDeveloper,
  Brave,
  Zen,
  TorBrowser,
}

impl BrowserType {
  pub fn as_str(&self) -> &'static str {
    match self {
      BrowserType::MullvadBrowser => "mullvad-browser",
      BrowserType::Chromium => "chromium",
      BrowserType::Firefox => "firefox",
      BrowserType::FirefoxDeveloper => "firefox-developer",
      BrowserType::Brave => "brave",
      BrowserType::Zen => "zen",
      BrowserType::TorBrowser => "tor-browser",
    }
  }

  pub fn from_str(s: &str) -> Result<Self, String> {
    match s {
      "mullvad-browser" => Ok(BrowserType::MullvadBrowser),
      "chromium" => Ok(BrowserType::Chromium),
      "firefox" => Ok(BrowserType::Firefox),
      "firefox-developer" => Ok(BrowserType::FirefoxDeveloper),
      "brave" => Ok(BrowserType::Brave),
      "zen" => Ok(BrowserType::Zen),
      "tor-browser" => Ok(BrowserType::TorBrowser),
      _ => Err(format!("Unknown browser type: {s}")),
    }
  }
}

pub trait Browser: Send + Sync {
  fn browser_type(&self) -> BrowserType;
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>>;
  fn create_launch_args(
    &self,
    profile_path: &str,
    _proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>>;
  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool;
}

pub struct FirefoxBrowser {
  browser_type: BrowserType,
}

impl FirefoxBrowser {
  pub fn new(browser_type: BrowserType) -> Self {
    Self { browser_type }
  }
}

impl Browser for FirefoxBrowser {
  fn browser_type(&self) -> BrowserType {
    self.browser_type.clone()
  }

  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Find the .app directory
    let app_path = std::fs::read_dir(install_dir)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("Browser app not found")?;

    // Construct the browser executable path
    let mut executable_dir = app_path.path();
    executable_dir.push("Contents");
    executable_dir.push("MacOS");

    // Find the first executable in the MacOS directory
    let executable_path = std::fs::read_dir(&executable_dir)?
      .filter_map(Result::ok)
      .find(|entry| {
        let binding = entry.file_name();
        let name = binding.to_string_lossy();
        name.starts_with("firefox")
          || name.starts_with("mullvad")
          || name.starts_with("zen")
          || name.starts_with("tor")
          || name.contains("Browser")
      })
      .map(|entry| entry.path())
      .ok_or("No executable found in MacOS directory")?;

    Ok(executable_path)
  }

  fn create_launch_args(
    &self,
    profile_path: &str,
    _proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut args = vec!["-profile".to_string(), profile_path.to_string()];

    // Only use -no-remote for browsers that require it for security (Mullvad, Tor)
    // Regular Firefox browsers can use remote commands for better URL handling
    match self.browser_type {
      BrowserType::MullvadBrowser | BrowserType::TorBrowser => {
        args.push("-no-remote".to_string());
      }
      BrowserType::Firefox | BrowserType::FirefoxDeveloper | BrowserType::Zen => {
        // Don't use -no-remote so we can communicate with existing instances
      }
      _ => {}
    }

    // Firefox-based browsers use profile directory and user.js for proxy configuration
    if let Some(url) = url {
      args.push(url);
    }

    Ok(args)
  }

  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool {
    let browser_dir = binaries_dir
      .join(self.browser_type().as_str())
      .join(version);

    println!("Firefox browser checking version {version} in directory: {browser_dir:?}");

    // Only check if directory exists and contains a .app file
    if browser_dir.exists() {
      println!("Directory exists, checking for .app files...");
      if let Ok(entries) = std::fs::read_dir(&browser_dir) {
        for entry in entries.flatten() {
          println!("  Found entry: {:?}", entry.path());
          if entry.path().extension().is_some_and(|ext| ext == "app") {
            println!("  Found .app file: {:?}", entry.path());
            return true;
          }
        }
      }
      println!("No .app files found in directory");
    } else {
      println!("Directory does not exist: {browser_dir:?}");
    }
    false
  }
}

// Chromium-based browsers (Chromium, Brave)
pub struct ChromiumBrowser {
  browser_type: BrowserType,
}

impl ChromiumBrowser {
  pub fn new(browser_type: BrowserType) -> Self {
    Self { browser_type }
  }
}

impl Browser for ChromiumBrowser {
  fn browser_type(&self) -> BrowserType {
    self.browser_type.clone()
  }

  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Find the .app directory
    let app_path = std::fs::read_dir(install_dir)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("Browser app not found")?;

    // Construct the browser executable path
    let mut executable_dir = app_path.path();
    executable_dir.push("Contents");
    executable_dir.push("MacOS");

    // Find the first executable in the MacOS directory
    let executable_path = std::fs::read_dir(&executable_dir)?
      .filter_map(Result::ok)
      .find(|entry| {
        let binding = entry.file_name();
        let name = binding.to_string_lossy();
        name.contains("Chromium") || name.contains("Brave") || name.contains("Google Chrome")
      })
      .map(|entry| entry.path())
      .ok_or("No executable found in MacOS directory")?;

    Ok(executable_path)
  }

  fn create_launch_args(
    &self,
    profile_path: &str,
    proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut args = vec![
      format!("--user-data-dir={}", profile_path),
      "--no-default-browser-check".to_string(),
      "--disable-background-mode".to_string(),
      "--disable-component-update".to_string(),
      "--disable-background-timer-throttling".to_string(),
      "--crash-server-url=".to_string(),
    ];

    // Add proxy configuration if provided
    if let Some(proxy) = proxy_settings {
      if proxy.enabled {
        let pac_path = Path::new(profile_path).join("proxy.pac");
        if pac_path.exists() {
          let pac_content = fs::read(&pac_path)?;
          let pac_base64 = general_purpose::STANDARD.encode(&pac_content);
          args.push(format!(
            "--proxy-pac-url=data:application/x-javascript-config;base64,{pac_base64}"
          ));
        }
      }
    }

    if let Some(url) = url {
      args.push(url);
    }

    Ok(args)
  }

  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool {
    let browser_dir = binaries_dir
      .join(self.browser_type().as_str())
      .join(version);

    println!("Chromium browser checking version {version} in directory: {browser_dir:?}");

    // Check if directory exists and contains at least one .app file
    if browser_dir.exists() {
      println!("Directory exists, checking for .app files...");
      if let Ok(entries) = std::fs::read_dir(&browser_dir) {
        for entry in entries.flatten() {
          println!("  Found entry: {:?}", entry.path());
          if entry.path().extension().is_some_and(|ext| ext == "app") {
            println!("  Found .app file: {:?}", entry.path());
            // Try to get the executable path as a final verification
            if self.get_executable_path(&browser_dir).is_ok() {
              println!("  Executable path verification successful");
              return true;
            } else {
              println!("  Executable path verification failed");
            }
          }
        }
      }
      println!("No valid .app files found in directory");
    } else {
      println!("Directory does not exist: {browser_dir:?}");
    }
    false
  }
}

// Factory function to create browser instances
pub fn create_browser(browser_type: BrowserType) -> Box<dyn Browser> {
  match browser_type {
    BrowserType::MullvadBrowser
    | BrowserType::Firefox
    | BrowserType::FirefoxDeveloper
    | BrowserType::Zen
    | BrowserType::TorBrowser => Box::new(FirefoxBrowser::new(browser_type)),
    BrowserType::Chromium | BrowserType::Brave => Box::new(ChromiumBrowser::new(browser_type)),
  }
}

// Add GithubRelease and GithubAsset structs to browser.rs if they don't already exist
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GithubRelease {
  pub tag_name: String,
  #[serde(default)]
  pub name: String,
  pub assets: Vec<GithubAsset>,
  #[serde(default)]
  pub published_at: String,
  #[serde(default)]
  pub is_alpha: bool,
  #[serde(default)]
  pub prerelease: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GithubAsset {
  pub name: String,
  pub browser_download_url: String,
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  #[test]
  fn test_browser_type_conversions() {
    // Test as_str
    assert_eq!(BrowserType::MullvadBrowser.as_str(), "mullvad-browser");
    assert_eq!(BrowserType::Firefox.as_str(), "firefox");
    assert_eq!(BrowserType::FirefoxDeveloper.as_str(), "firefox-developer");
    assert_eq!(BrowserType::Chromium.as_str(), "chromium");
    assert_eq!(BrowserType::Brave.as_str(), "brave");
    assert_eq!(BrowserType::Zen.as_str(), "zen");
    assert_eq!(BrowserType::TorBrowser.as_str(), "tor-browser");

    // Test from_str
    assert_eq!(
      BrowserType::from_str("mullvad-browser").unwrap(),
      BrowserType::MullvadBrowser
    );
    assert_eq!(
      BrowserType::from_str("firefox").unwrap(),
      BrowserType::Firefox
    );
    assert_eq!(
      BrowserType::from_str("firefox-developer").unwrap(),
      BrowserType::FirefoxDeveloper
    );
    assert_eq!(
      BrowserType::from_str("chromium").unwrap(),
      BrowserType::Chromium
    );
    assert_eq!(BrowserType::from_str("brave").unwrap(), BrowserType::Brave);
    assert_eq!(BrowserType::from_str("zen").unwrap(), BrowserType::Zen);
    assert_eq!(
      BrowserType::from_str("tor-browser").unwrap(),
      BrowserType::TorBrowser
    );

    // Test invalid browser type
    assert!(BrowserType::from_str("invalid").is_err());
    assert!(BrowserType::from_str("").is_err());
    assert!(BrowserType::from_str("Firefox").is_err()); // Case sensitive
  }

  #[test]
  fn test_firefox_browser_creation() {
    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    assert_eq!(browser.browser_type(), BrowserType::Firefox);

    let browser = FirefoxBrowser::new(BrowserType::MullvadBrowser);
    assert_eq!(browser.browser_type(), BrowserType::MullvadBrowser);

    let browser = FirefoxBrowser::new(BrowserType::TorBrowser);
    assert_eq!(browser.browser_type(), BrowserType::TorBrowser);

    let browser = FirefoxBrowser::new(BrowserType::Zen);
    assert_eq!(browser.browser_type(), BrowserType::Zen);
  }

  #[test]
  fn test_chromium_browser_creation() {
    let browser = ChromiumBrowser::new(BrowserType::Chromium);
    assert_eq!(browser.browser_type(), BrowserType::Chromium);

    let browser = ChromiumBrowser::new(BrowserType::Brave);
    assert_eq!(browser.browser_type(), BrowserType::Brave);
  }

  #[test]
  fn test_browser_factory() {
    // Test Firefox-based browsers
    let browser = create_browser(BrowserType::Firefox);
    assert_eq!(browser.browser_type(), BrowserType::Firefox);

    let browser = create_browser(BrowserType::MullvadBrowser);
    assert_eq!(browser.browser_type(), BrowserType::MullvadBrowser);

    let browser = create_browser(BrowserType::Zen);
    assert_eq!(browser.browser_type(), BrowserType::Zen);

    let browser = create_browser(BrowserType::TorBrowser);
    assert_eq!(browser.browser_type(), BrowserType::TorBrowser);

    let browser = create_browser(BrowserType::FirefoxDeveloper);
    assert_eq!(browser.browser_type(), BrowserType::FirefoxDeveloper);

    // Test Chromium-based browsers
    let browser = create_browser(BrowserType::Chromium);
    assert_eq!(browser.browser_type(), BrowserType::Chromium);

    let browser = create_browser(BrowserType::Brave);
    assert_eq!(browser.browser_type(), BrowserType::Brave);
  }

  #[test]
  fn test_firefox_launch_args() {
    // Test regular Firefox (should not use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile"]);
    assert!(!args.contains(&"-no-remote".to_string()));

    let args = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
      )
      .unwrap();
    assert_eq!(
      args,
      vec!["-profile", "/path/to/profile", "https://example.com"]
    );

    // Test Mullvad Browser (should use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::MullvadBrowser);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile", "-no-remote"]);

    // Test Tor Browser (should use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::TorBrowser);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile", "-no-remote"]);

    // Test Zen Browser (should not use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::Zen);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile"]);
    assert!(!args.contains(&"-no-remote".to_string()));
  }

  #[test]
  fn test_chromium_launch_args() {
    let browser = ChromiumBrowser::new(BrowserType::Chromium);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();

    // Test that basic required arguments are present
    assert!(args.contains(&"--user-data-dir=/path/to/profile".to_string()));
    assert!(args.contains(&"--no-default-browser-check".to_string()));

    // Test that automatic update disabling arguments are present
    assert!(args.contains(&"--disable-background-mode".to_string()));
    assert!(args.contains(&"--disable-component-update".to_string()));

    let args_with_url = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
      )
      .unwrap();
    assert!(args_with_url.contains(&"https://example.com".to_string()));

    // Verify URL is at the end
    assert_eq!(args_with_url.last().unwrap(), "https://example.com");
  }

  #[test]
  fn test_proxy_settings_creation() {
    let proxy = ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
    };

    assert!(proxy.enabled);
    assert_eq!(proxy.proxy_type, "http");
    assert_eq!(proxy.host, "127.0.0.1");
    assert_eq!(proxy.port, 8080);

    // Test different proxy types
    let socks_proxy = ProxySettings {
      enabled: true,
      proxy_type: "socks5".to_string(),
      host: "proxy.example.com".to_string(),
      port: 1080,
    };

    assert_eq!(socks_proxy.proxy_type, "socks5");
    assert_eq!(socks_proxy.host, "proxy.example.com");
    assert_eq!(socks_proxy.port, 1080);
  }

  #[test]
  fn test_version_downloaded_check() {
    let temp_dir = TempDir::new().unwrap();
    let binaries_dir = temp_dir.path();

    // Create a mock Firefox browser installation
    let browser_dir = binaries_dir.join("firefox").join("139.0");
    fs::create_dir_all(&browser_dir).unwrap();

    // Create a mock .app directory
    let app_dir = browser_dir.join("Firefox.app");
    fs::create_dir_all(&app_dir).unwrap();

    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    assert!(browser.is_version_downloaded("139.0", binaries_dir));
    assert!(!browser.is_version_downloaded("140.0", binaries_dir));

    // Test with Chromium browser
    let chromium_dir = binaries_dir.join("chromium").join("1465660");
    fs::create_dir_all(&chromium_dir).unwrap();
    let chromium_app_dir = chromium_dir.join("Chromium.app");
    fs::create_dir_all(chromium_app_dir.join("Contents").join("MacOS")).unwrap();

    // Create a mock executable
    let executable_path = chromium_app_dir
      .join("Contents")
      .join("MacOS")
      .join("Chromium");
    fs::write(&executable_path, "mock executable").unwrap();

    let chromium_browser = ChromiumBrowser::new(BrowserType::Chromium);
    assert!(chromium_browser.is_version_downloaded("1465660", binaries_dir));
    assert!(!chromium_browser.is_version_downloaded("1465661", binaries_dir));
  }

  #[test]
  fn test_version_downloaded_no_app_directory() {
    let temp_dir = TempDir::new().unwrap();
    let binaries_dir = temp_dir.path();

    // Create browser directory but no .app directory
    let browser_dir = binaries_dir.join("firefox").join("139.0");
    fs::create_dir_all(&browser_dir).unwrap();

    // Create some other files but no .app
    fs::write(browser_dir.join("readme.txt"), "Some content").unwrap();

    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    assert!(!browser.is_version_downloaded("139.0", binaries_dir));
  }

  #[test]
  fn test_browser_type_clone_and_debug() {
    let browser_type = BrowserType::Firefox;
    let cloned = browser_type.clone();
    assert_eq!(browser_type, cloned);

    // Test Debug trait
    let debug_str = format!("{browser_type:?}");
    assert!(debug_str.contains("Firefox"));
  }

  #[test]
  fn test_proxy_settings_serialization() {
    let proxy = ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
    };

    // Test that it can be serialized (implements Serialize)
    let json = serde_json::to_string(&proxy).unwrap();
    assert!(json.contains("127.0.0.1"));
    assert!(json.contains("8080"));
    assert!(json.contains("http"));

    // Test that it can be deserialized (implements Deserialize)
    let deserialized: ProxySettings = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.enabled, proxy.enabled);
    assert_eq!(deserialized.proxy_type, proxy.proxy_type);
    assert_eq!(deserialized.host, proxy.host);
    assert_eq!(deserialized.port, proxy.port);
  }
}
