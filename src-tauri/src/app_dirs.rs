use directories::BaseDirs;
use std::path::PathBuf;
use std::sync::OnceLock;

static BASE_DIRS: OnceLock<BaseDirs> = OnceLock::new();

fn base_dirs() -> &'static BaseDirs {
  BASE_DIRS.get_or_init(|| BaseDirs::new().expect("Failed to get base directories"))
}

pub fn app_name() -> &'static str {
  if cfg!(debug_assertions) {
    "DonutBrowserDev"
  } else {
    "DonutBrowser"
  }
}

pub fn data_dir() -> PathBuf {
  #[cfg(test)]
  {
    if let Some(dir) = TEST_DATA_DIR.with(|cell| cell.borrow().clone()) {
      return dir;
    }
  }

  if let Ok(dir) = std::env::var("DONUTBROWSER_DATA_DIR") {
    return PathBuf::from(dir);
  }

  base_dirs().data_local_dir().join(app_name())
}

pub fn cache_dir() -> PathBuf {
  #[cfg(test)]
  {
    if let Some(dir) = TEST_CACHE_DIR.with(|cell| cell.borrow().clone()) {
      return dir;
    }
  }

  if let Ok(dir) = std::env::var("DONUTBROWSER_CACHE_DIR") {
    return PathBuf::from(dir);
  }

  base_dirs().cache_dir().join(app_name())
}

pub fn profiles_dir() -> PathBuf {
  data_dir().join("profiles")
}

pub fn binaries_dir() -> PathBuf {
  data_dir().join("binaries")
}

pub fn data_subdir() -> PathBuf {
  data_dir().join("data")
}

pub fn settings_dir() -> PathBuf {
  data_dir().join("settings")
}

pub fn proxies_dir() -> PathBuf {
  data_dir().join("proxies")
}

pub fn vpn_dir() -> PathBuf {
  data_dir().join("vpn")
}

pub fn extensions_dir() -> PathBuf {
  data_dir().join("extensions")
}

#[cfg(test)]
thread_local! {
  static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
  static TEST_CACHE_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub struct TestDirGuard {
  kind: TestDirKind,
}

#[cfg(test)]
enum TestDirKind {
  Data,
  Cache,
}

#[cfg(test)]
impl Drop for TestDirGuard {
  fn drop(&mut self) {
    match self.kind {
      TestDirKind::Data => TEST_DATA_DIR.with(|cell| *cell.borrow_mut() = None),
      TestDirKind::Cache => TEST_CACHE_DIR.with(|cell| *cell.borrow_mut() = None),
    }
  }
}

#[cfg(test)]
pub fn set_test_data_dir(dir: PathBuf) -> TestDirGuard {
  TEST_DATA_DIR.with(|cell| *cell.borrow_mut() = Some(dir));
  TestDirGuard {
    kind: TestDirKind::Data,
  }
}

#[cfg(test)]
pub fn set_test_cache_dir(dir: PathBuf) -> TestDirGuard {
  TEST_CACHE_DIR.with(|cell| *cell.borrow_mut() = Some(dir));
  TestDirGuard {
    kind: TestDirKind::Cache,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_app_name() {
    let name = app_name();
    assert!(
      name == "DonutBrowser" || name == "DonutBrowserDev",
      "app_name should be DonutBrowser or DonutBrowserDev, got: {name}"
    );
  }

  #[test]
  fn test_data_dir_returns_path() {
    let dir = data_dir();
    assert!(
      dir.to_string_lossy().contains(app_name()),
      "data_dir should contain app_name"
    );
  }

  #[test]
  fn test_cache_dir_returns_path() {
    let dir = cache_dir();
    assert!(
      dir.to_string_lossy().contains(app_name()),
      "cache_dir should contain app_name"
    );
  }

  #[test]
  fn test_subdirectory_helpers() {
    assert!(profiles_dir().ends_with("profiles"));
    assert!(binaries_dir().ends_with("binaries"));
    assert!(data_subdir().ends_with("data"));
    assert!(settings_dir().ends_with("settings"));
    assert!(proxies_dir().ends_with("proxies"));
    assert!(vpn_dir().ends_with("vpn"));
    assert!(extensions_dir().ends_with("extensions"));
  }

  #[test]
  fn test_set_test_data_dir() {
    let tmp = PathBuf::from("/tmp/test-donut-data");
    let _guard = set_test_data_dir(tmp.clone());
    assert_eq!(data_dir(), tmp);
    assert_eq!(profiles_dir(), tmp.join("profiles"));
    assert_eq!(binaries_dir(), tmp.join("binaries"));
  }

  #[test]
  fn test_set_test_cache_dir() {
    let tmp = PathBuf::from("/tmp/test-donut-cache");
    let _guard = set_test_cache_dir(tmp.clone());
    assert_eq!(cache_dir(), tmp);
  }

  #[test]
  fn test_guard_cleanup() {
    let original_data = data_dir();
    let original_cache = cache_dir();

    {
      let _guard = set_test_data_dir(PathBuf::from("/tmp/test-cleanup-data"));
      assert_eq!(data_dir(), PathBuf::from("/tmp/test-cleanup-data"));
    }
    assert_eq!(data_dir(), original_data);

    {
      let _guard = set_test_cache_dir(PathBuf::from("/tmp/test-cleanup-cache"));
      assert_eq!(cache_dir(), PathBuf::from("/tmp/test-cleanup-cache"));
    }
    assert_eq!(cache_dir(), original_cache);
  }
}
