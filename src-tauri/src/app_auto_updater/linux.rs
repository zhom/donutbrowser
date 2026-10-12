use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinuxInstallationMethod {
  Deb,
  Rpm,
  AppImage,
  ManagedExternally,
  Manual,
  Unknown,
}

/// An ownership result is None when the tool is unavailable. Merely having
/// dpkg/rpm installed does not establish ownership of the running executable.
#[derive(Default)]
pub(super) struct InstallationFacts<'a> {
  pub exe: Option<&'a Path>,
  pub appimage: bool,
  pub flatpak: bool,
  pub snap: Option<&'a Path>,
  pub dpkg_owned: Option<bool>,
  pub rpm_owned: Option<bool>,
  pub pacman_owned: Option<bool>,
  pub deb_repo: bool,
}

pub(super) fn detect_installation(facts: &InstallationFacts<'_>) -> LinuxInstallationMethod {
  use LinuxInstallationMethod::*;
  if facts.flatpak
    || facts.exe.is_some_and(|exe| {
      exe.starts_with("/nix/store")
        || facts
          .snap
          .is_some_and(|snap| !snap.as_os_str().is_empty() && exe.starts_with(snap))
    })
    || facts.pacman_owned == Some(true)
  {
    return ManagedExternally;
  }
  if facts.appimage {
    return AppImage;
  }
  let Some(exe) = facts.exe else {
    return Unknown;
  };
  if facts.dpkg_owned == Some(true) {
    return Deb;
  }
  if facts.rpm_owned == Some(true) {
    return Rpm;
  }
  if facts.deb_repo && facts.dpkg_owned.is_some() {
    return Deb;
  }
  if exe.starts_with("/usr/bin") || exe.starts_with("/usr/local/bin") || exe.starts_with("/home") {
    Manual
  } else {
    Unknown
  }
}

impl LinuxInstallationMethod {
  pub(super) fn package_extension(self) -> Option<&'static str> {
    match self {
      Self::Deb => Some("deb"),
      Self::Rpm => Some("rpm"),
      _ => None,
    }
  }

  /// The frontend must explicitly opt in after a click; metadata supplied by
  /// a caller cannot enable installation for an unsupported local install.
  pub(super) fn validate_update(
    self,
    repo_update: bool,
    extension: Option<&str>,
    user_initiated: bool,
  ) -> Result<(), &'static str> {
    if repo_update || self.package_extension().is_none() || self.package_extension() != extension {
      return Err("UPDATE_MANUAL_REQUIRED");
    }
    if !user_initiated {
      return Err("UPDATE_CONFIRMATION_REQUIRED");
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn detects_installations_without_querying_the_host() {
    use LinuxInstallationMethod::*;
    let cases = [
      (
        InstallationFacts {
          dpkg_owned: Some(true),
          ..system()
        },
        Deb,
      ),
      (
        InstallationFacts {
          rpm_owned: Some(true),
          ..system()
        },
        Rpm,
      ),
      (
        InstallationFacts {
          pacman_owned: Some(true),
          ..system()
        },
        ManagedExternally,
      ),
      (system(), Manual),
      (
        InstallationFacts {
          dpkg_owned: Some(false),
          rpm_owned: Some(false),
          ..system()
        },
        Manual,
      ),
      (
        InstallationFacts {
          appimage: true,
          ..system()
        },
        AppImage,
      ),
      (
        InstallationFacts {
          exe: Some(Path::new("/nix/store/abc-donut/bin/donutbrowser")),
          dpkg_owned: Some(true),
          ..system()
        },
        ManagedExternally,
      ),
      (
        InstallationFacts {
          flatpak: true,
          ..system()
        },
        ManagedExternally,
      ),
      (
        InstallationFacts {
          exe: Some(Path::new("/snap/donut/current/usr/bin/donutbrowser")),
          snap: Some(Path::new("/snap/donut/current")),
          ..system()
        },
        ManagedExternally,
      ),
      // A snapped terminal can leak SNAP to an ordinary installation.
      (
        InstallationFacts {
          snap: Some(Path::new("/snap/terminal/current")),
          dpkg_owned: Some(true),
          ..system()
        },
        Deb,
      ),
      (
        InstallationFacts {
          snap: Some(Path::new("")),
          ..system()
        },
        Manual,
      ),
      (
        InstallationFacts {
          deb_repo: true,
          dpkg_owned: Some(false),
          ..system()
        },
        Deb,
      ),
      (
        InstallationFacts {
          deb_repo: true,
          ..system()
        },
        Manual,
      ),
      (
        InstallationFacts {
          deb_repo: true,
          dpkg_owned: Some(true),
          pacman_owned: Some(true),
          ..system()
        },
        ManagedExternally,
      ),
      (
        InstallationFacts {
          deb_repo: true,
          dpkg_owned: Some(false),
          rpm_owned: Some(true),
          ..system()
        },
        Rpm,
      ),
      (InstallationFacts::default(), Unknown),
      (
        InstallationFacts {
          exe: Some(Path::new("/opt/donutbrowser")),
          ..system()
        },
        Unknown,
      ),
    ];
    for (facts, expected) in cases {
      assert_eq!(detect_installation(&facts), expected, "{:?}", facts.exe);
    }
  }

  fn system() -> InstallationFacts<'static> {
    InstallationFacts {
      exe: Some(Path::new("/usr/bin/donutbrowser")),
      ..Default::default()
    }
  }

  #[test]
  fn unsupported_installs_never_accept_packages_even_after_a_click() {
    use LinuxInstallationMethod::*;
    for method in [ManagedExternally, Manual, Unknown, AppImage] {
      assert_eq!(method.package_extension(), None);
      for extension in ["deb", "rpm", "gz", "AppImage"] {
        for user_initiated in [false, true] {
          assert_eq!(
            method.validate_update(false, Some(extension), user_initiated),
            Err("UPDATE_MANUAL_REQUIRED")
          );
        }
      }
    }
  }

  #[test]
  fn supported_packages_require_a_click_and_the_matching_package_type() {
    for (method, extension, wrong) in [
      (LinuxInstallationMethod::Deb, "deb", "rpm"),
      (LinuxInstallationMethod::Rpm, "rpm", "deb"),
    ] {
      assert_eq!(
        method.validate_update(false, Some(extension), false),
        Err("UPDATE_CONFIRMATION_REQUIRED")
      );
      assert_eq!(method.validate_update(false, Some(extension), true), Ok(()));
      assert_eq!(
        method.validate_update(false, Some(wrong), true),
        Err("UPDATE_MANUAL_REQUIRED")
      );
      assert_eq!(
        method.validate_update(true, Some(extension), true),
        Err("UPDATE_MANUAL_REQUIRED")
      );
      assert_eq!(
        method.validate_update(false, None, true),
        Err("UPDATE_MANUAL_REQUIRED")
      );
    }
  }
}
