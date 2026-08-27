use serde::Serialize;
use tauri::command;

pub struct DefaultBrowser {}

/// What happened when the user asked Donut to become the default browser.
///
/// macOS and Linux let a program make the change itself. Windows does not. The
/// registry value that decides the handler carries a signature only the shell
/// can produce, so the most a program may do is register itself and open the
/// page where the user makes the choice. Without this distinction the caller
/// reports a change that has not happened yet, which is what the Windows path
/// used to do.
///
/// Each platform builds exactly one of these, so on any single target the other
/// one reads as never constructed. That is what the allow is for: the variant is
/// live, just not on the host being compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
#[allow(dead_code)]
pub enum SetDefaultOutcome {
  /// Donut is the default browser now. Nothing is left for the user to do.
  Set,
  /// Registration is complete and the system settings page is open. The user
  /// makes the final choice there.
  AwaitingSystemSettings,
}

impl DefaultBrowser {
  fn new() -> Self {
    Self {}
  }

  pub fn instance() -> &'static DefaultBrowser {
    &DEFAULT_BROWSER
  }

  pub async fn is_default_browser(&self) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    return macos::is_default_browser();

    #[cfg(target_os = "windows")]
    return windows::is_default_browser();

    // Linux answers this by running `xdg-mime`, a shell script that forks
    // further. That is blocking work with no upper bound, and this command
    // runs on the same async runtime as every other command, the REST API and
    // the sync scheduler, so doing it inline occupies a worker for as long as
    // the desktop takes to answer. The Settings page polls this on a timer.
    #[cfg(target_os = "linux")]
    return blocking(linux::is_default_browser).await;

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("Unsupported platform".to_string())
  }

  pub async fn set_as_default_browser(&self) -> Result<SetDefaultOutcome, String> {
    #[cfg(target_os = "macos")]
    return macos::set_as_default_browser().map(|()| SetDefaultOutcome::Set);

    // Windows writes several registry trees, broadcasts `WM_SETTINGCHANGE` to
    // every top-level window on the desktop and then hands off to the shell.
    // The broadcast alone costs about 130 ms on an idle desktop and seconds on
    // a busy one, so this does not belong on a runtime worker either.
    #[cfg(target_os = "windows")]
    return blocking(windows::set_as_default_browser).await;

    // Same reasoning, and this one additionally sleeps 500ms before verifying.
    #[cfg(target_os = "linux")]
    return blocking(linux::set_as_default_browser)
      .await
      .map(|()| SetDefaultOutcome::Set);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("Unsupported platform".to_string())
  }
}

/// Run blocking work off the async runtime's worker threads.
#[cfg(any(target_os = "linux", target_os = "windows"))]
async fn blocking<T, F>(work: F) -> Result<T, String>
where
  F: FnOnce() -> Result<T, String> + Send + 'static,
  T: Send + 'static,
{
  tokio::task::spawn_blocking(work)
    .await
    .map_err(|e| format!("Default browser check did not run: {e}"))?
}

#[cfg(target_os = "macos")]
mod macos {
  use core_foundation::base::OSStatus;
  use core_foundation::string::CFStringRef;
  use core_foundation::{base::TCFType, string::CFString};

  #[link(name = "CoreServices", kind = "framework")]
  extern "C" {
    fn LSSetDefaultHandlerForURLScheme(scheme: CFStringRef, bundle_id: CFStringRef) -> OSStatus;
    fn LSCopyDefaultHandlerForURLScheme(scheme: CFStringRef) -> CFStringRef;
  }

  pub fn is_default_browser() -> Result<bool, String> {
    let schemes = ["http", "https"];
    let bundle_id = "com.donutbrowser";

    for scheme in schemes {
      let scheme_str = CFString::new(scheme);
      unsafe {
        let current_handler = LSCopyDefaultHandlerForURLScheme(scheme_str.as_concrete_TypeRef());
        if current_handler.is_null() {
          return Ok(false);
        }

        let current_handler_cf = CFString::wrap_under_create_rule(current_handler);
        let current_handler_str = current_handler_cf.to_string();

        if current_handler_str != bundle_id {
          return Ok(false);
        }
      }
    }
    Ok(true)
  }

  pub fn set_as_default_browser() -> Result<(), String> {
    let bundle_id = CFString::new("com.donutbrowser");
    let schemes = ["http", "https"];

    for scheme in schemes {
      let scheme_str = CFString::new(scheme);
      unsafe {
        let status = LSSetDefaultHandlerForURLScheme(
          scheme_str.as_concrete_TypeRef(),
          bundle_id.as_concrete_TypeRef(),
        );
        if status != 0 {
          let error_msg = match status {
            -54 => format!(
              "Failed to set as default browser for scheme '{scheme}'. The app is not properly registered as a browser. Please:\n1. Build and install the app properly\n2. Manually set Donut Browser as default in System Settings > General > Default web browser\n3. Make sure the app is in your Applications folder"
            ),
            _ => format!(
              "Failed to set as default browser for scheme '{scheme}'. Status code: {status}. Please manually set Donut Browser as default in System Settings > General > Default web browser."
            )
          };
          return Err(error_msg);
        }
      }
    }
    Ok(())
  }
}

#[cfg(target_os = "windows")]
#[allow(clippy::needless_borrows_for_generic_args)]
mod windows {
  use super::SetDefaultOutcome;
  use std::path::Path;
  use winreg::enums::*;
  use winreg::RegKey;

  /// The key Windows knows us by. Never shown to a person.
  const APP_NAME: &str = "DonutBrowser";
  /// The name Windows shows in "Default apps" and in "Open with".
  const DISPLAY_NAME: &str = "Donut Browser";
  const DESCRIPTION: &str = "Donut Browser - Simple Yet Powerful Anti-Detect Browser";
  const PROG_ID: &str = "DonutBrowser.HTML";

  /// A web browser registers under `StartMenuInternet`, and
  /// `RegisteredApplications` points at the `Capabilities` subkey of that
  /// entry. Edge, Chrome and Firefox all do exactly this, and the shell reads
  /// the capability data from there.
  ///
  /// The previous layout invented its own key at `Software\DonutBrowser` and
  /// pointed `RegisteredApplications` at the parent instead of at
  /// `Capabilities`. Every other entry on a normal machine ends in
  /// `Capabilities`. The shell found no capability data, so Donut was never
  /// offered as a browser and the button appeared to do nothing.
  const CLIENT_KEY: &str = r"Software\Clients\StartMenuInternet\DonutBrowser";
  /// The value written into `RegisteredApplications`.
  const CAPABILITIES_KEY: &str = r"Software\Clients\StartMenuInternet\DonutBrowser\Capabilities";
  /// The layout earlier builds wrote. Removed on every run, so a machine that
  /// ran one of those does not keep stale capability data claiming http.
  const LEGACY_APP_KEY: &str = r"Software\DonutBrowser";

  const URL_SCHEMES: [&str; 2] = ["http", "https"];
  /// The file types a browser is asked to open from Explorer. The ProgId
  /// command passes the path through as `%1`, and `urls_from_args` in `lib.rs`
  /// turns a path into a `file://` URL, so every extension listed here can
  /// actually be serviced. Do not add one that cannot.
  const FILE_EXTENSIONS: [&str; 4] = [".htm", ".html", ".shtml", ".xhtml"];

  pub fn is_default_browser() -> Result<bool, String> {
    for scheme in URL_SCHEMES {
      if !is_default_for_scheme(scheme)? {
        return Ok(false);
      }
    }

    Ok(true)
  }

  pub fn set_as_default_browser() -> Result<SetDefaultOutcome, String> {
    let exe_path =
      std::env::current_exe().map_err(|e| format!("Failed to get current executable path: {e}"))?;

    let exe_path = exe_path
      .to_str()
      .ok_or("Failed to convert executable path to string")?;

    if !Path::new(exe_path).exists() {
      return Err(format!("Executable not found at: {exe_path}"));
    }

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    remove_legacy_registration(&hkcu);
    register_prog_id(&hkcu, exe_path)?;
    register_client(&hkcu, exe_path)?;
    register_file_extensions(&hkcu)?;
    register_application(&hkcu)?;

    notify_system_of_changes();

    open_default_apps_settings()?;

    Ok(SetDefaultOutcome::AwaitingSystemSettings)
  }

  /// Wrap a path in the quotes the shell expects around a command or an icon.
  fn quoted(value: &str) -> String {
    format!(r#""{value}""#)
  }

  fn is_default_for_scheme(scheme: &str) -> Result<bool, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let path =
      format!(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice");

    match hkcu.open_subkey(&path) {
      Ok(key) => match key.get_value::<String, _>("ProgId") {
        Ok(prog_id) => Ok(prog_id == PROG_ID),
        Err(_) => Ok(false),
      },
      Err(_) => Ok(false),
    }
  }

  /// Delete the layout earlier builds wrote.
  ///
  /// Nothing else in this application has ever written under that key, so
  /// removing it cannot lose anything a user cares about. Leaving it would
  /// leave a second `Capabilities` block claiming http and https from a key the
  /// shell no longer reads.
  ///
  /// The old code also wrote the ProgId into the default value of
  /// `Software\Classes\.html` and `.htm`. That value is the association itself,
  /// and it was never ours to take. Give it back, but only where it still holds
  /// the ProgId we wrote. Any other value is the user's own choice and is left
  /// alone.
  fn remove_legacy_registration(root: &RegKey) {
    match root.delete_subkey_all(LEGACY_APP_KEY) {
      Ok(()) => log::debug!("Removed the superseded default-browser registration key"),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
      Err(e) => log::debug!("Could not remove the superseded registration key: {e}"),
    }

    for extension in [".htm", ".html"] {
      let path = format!(r"Software\Classes\{extension}");
      let Ok(key) = root.open_subkey_with_flags(&path, KEY_READ | KEY_SET_VALUE) else {
        continue;
      };

      let ours = key
        .get_value::<String, _>("")
        .map(|value| value == PROG_ID)
        .unwrap_or(false);

      if ours {
        match key.delete_value("") {
          Ok(()) => log::debug!("Released the {extension} association taken by an older build"),
          Err(e) => log::debug!("Could not release the {extension} association: {e}"),
        }
      }
    }
  }

  /// Describe the document type Donut opens, and how to open one.
  fn register_prog_id(root: &RegKey, exe_path: &str) -> Result<(), String> {
    let (prog_id_key, _) = root
      .create_subkey(format!(r"Software\Classes\{PROG_ID}"))
      .map_err(|e| format!("Failed to create ProgID key: {e}"))?;

    prog_id_key
      .set_value("", &"Donut Browser Document")
      .map_err(|e| format!("Failed to set ProgID default value: {e}"))?;

    prog_id_key
      .set_value("FriendlyTypeName", &"Donut Browser Document")
      .map_err(|e| format!("Failed to set FriendlyTypeName: {e}"))?;

    // The shell reads this block to put a name and an icon beside the ProgId in
    // the "Open with" list. Without it the entry shows as the raw ProgId.
    let (application, _) = prog_id_key
      .create_subkey("Application")
      .map_err(|e| format!("Failed to create ProgID Application key: {e}"))?;

    application
      .set_value("ApplicationName", &DISPLAY_NAME)
      .map_err(|e| format!("Failed to set ProgID ApplicationName: {e}"))?;

    application
      .set_value("ApplicationIcon", &format!("{},0", quoted(exe_path)))
      .map_err(|e| format!("Failed to set ProgID ApplicationIcon: {e}"))?;

    let (icon_key, _) = prog_id_key
      .create_subkey("DefaultIcon")
      .map_err(|e| format!("Failed to create DefaultIcon key: {e}"))?;

    icon_key
      .set_value("", &format!("{},0", quoted(exe_path)))
      .map_err(|e| format!("Failed to set default icon: {e}"))?;

    let (command_key, _) = prog_id_key
      .create_subkey(r"shell\open\command")
      .map_err(|e| format!("Failed to create command key: {e}"))?;

    command_key
      .set_value("", &format!(r#"{} "%1""#, quoted(exe_path)))
      .map_err(|e| format!("Failed to set command: {e}"))?;

    Ok(())
  }

  /// The `StartMenuInternet` entry: the shape the shell reads for a web
  /// browser. A display name, an icon, the command that starts it, the
  /// `InstallInfo` block the default-programs page expects, and the capability
  /// lists that say which schemes and file types it handles.
  fn register_client(root: &RegKey, exe_path: &str) -> Result<(), String> {
    let (client, _) = root
      .create_subkey(CLIENT_KEY)
      .map_err(|e| format!("Failed to create browser client key: {e}"))?;

    client
      .set_value("", &DISPLAY_NAME)
      .map_err(|e| format!("Failed to set client display name: {e}"))?;

    let (icon, _) = client
      .create_subkey("DefaultIcon")
      .map_err(|e| format!("Failed to create client DefaultIcon key: {e}"))?;

    icon
      .set_value("", &format!("{},0", quoted(exe_path)))
      .map_err(|e| format!("Failed to set client icon: {e}"))?;

    let (command, _) = client
      .create_subkey(r"shell\open\command")
      .map_err(|e| format!("Failed to create client command key: {e}"))?;

    // No `%1` here. This entry is how the shell starts the browser with no
    // document, for example from the Start menu.
    command
      .set_value("", &quoted(exe_path))
      .map_err(|e| format!("Failed to set client command: {e}"))?;

    // The shell reads the icons-visible state from here, so the block has to
    // exist. It also understands `ReinstallCommand`, `HideIconsCommand` and
    // `ShowIconsCommand`, and Edge and Chrome advertise all three. Donut does
    // not, because it does not act on `--make-default-browser`, `--hide-icons`
    // or `--show-icons`. Advertising a command the program ignores is the same
    // empty claim as registering a file type nothing can open. Add them here on
    // the day the flags do something.
    let (install_info, _) = client
      .create_subkey("InstallInfo")
      .map_err(|e| format!("Failed to create InstallInfo key: {e}"))?;

    install_info
      .set_value("IconsVisible", &1u32)
      .map_err(|e| format!("Failed to set IconsVisible: {e}"))?;

    let (capabilities, _) = client
      .create_subkey("Capabilities")
      .map_err(|e| format!("Failed to create Capabilities key: {e}"))?;

    // `ApplicationName` belongs inside `Capabilities`. The old code wrote it one
    // level up, where the shell does not look, so the entry had no name.
    capabilities
      .set_value("ApplicationName", &DISPLAY_NAME)
      .map_err(|e| format!("Failed to set ApplicationName: {e}"))?;

    capabilities
      .set_value("ApplicationDescription", &DESCRIPTION)
      .map_err(|e| format!("Failed to set ApplicationDescription: {e}"))?;

    capabilities
      .set_value("ApplicationIcon", &format!("{},0", quoted(exe_path)))
      .map_err(|e| format!("Failed to set ApplicationIcon: {e}"))?;

    let (url_assoc, _) = capabilities
      .create_subkey("URLAssociations")
      .map_err(|e| format!("Failed to create URLAssociations key: {e}"))?;

    for scheme in URL_SCHEMES {
      url_assoc
        .set_value(scheme, &PROG_ID)
        .map_err(|e| format!("Failed to set {scheme} association: {e}"))?;
    }

    let (file_assoc, _) = capabilities
      .create_subkey("FileAssociations")
      .map_err(|e| format!("Failed to create FileAssociations key: {e}"))?;

    for extension in FILE_EXTENSIONS {
      file_assoc
        .set_value(extension, &PROG_ID)
        .map_err(|e| format!("Failed to set {extension} association: {e}"))?;
    }

    Ok(())
  }

  /// Offer Donut in the "Open with" list for the HTML file types, without
  /// taking the association away from whatever the user already chose.
  ///
  /// The old code wrote the ProgId into the default value of
  /// `Software\Classes\.html`, which is the association itself. That replaced
  /// the user's choice without asking, was never undone on uninstall, and did
  /// not even take effect, because the per-user `FileExts` choice outranks it.
  /// `OpenWithProgids` is the additive form: it adds Donut to the list and
  /// displaces nothing.
  fn register_file_extensions(root: &RegKey) -> Result<(), String> {
    for extension in FILE_EXTENSIONS {
      let (open_with, _) = root
        .create_subkey(format!(r"Software\Classes\{extension}\OpenWithProgids"))
        .map_err(|e| format!("Failed to create OpenWithProgids key for {extension}: {e}"))?;

      // Only the value name matters here. The payload is a marker.
      open_with
        .set_value(PROG_ID, &"")
        .map_err(|e| format!("Failed to register the {extension} handler: {e}"))?;
    }

    Ok(())
  }

  /// Point `RegisteredApplications` at the capability data. This is what puts
  /// Donut in the list Windows offers under "Default apps".
  fn register_application(root: &RegKey) -> Result<(), String> {
    let (registered_apps, _) = root
      .create_subkey(r"Software\RegisteredApplications")
      .map_err(|e| format!("Failed to create RegisteredApplications key: {e}"))?;

    registered_apps
      .set_value(APP_NAME, &CAPABILITIES_KEY)
      .map_err(|e| format!("Failed to set registered application: {e}"))
  }

  /// Open the page where the user chooses the default browser.
  ///
  /// Windows does not let a program make itself the default. The value that
  /// decides the handler, the `UserChoice` key under `UrlAssociations`, carries
  /// a hash over the user's SID, the ProgId and a timestamp, and only the shell
  /// can produce it. Windows 11 also ships UCPD.sys, which blocks writes to
  /// those keys outright.
  ///
  /// The old code wrote `ProgId` there with no hash and discarded every error,
  /// then reported success. The registry never changed, the Settings page went
  /// on saying "Inactive", and the user was told nothing. Registration is the
  /// part a program is allowed to do. The choice belongs to the user, so open
  /// the page where they can make it and let the caller say so.
  fn open_default_apps_settings() -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{
      CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // `registeredAppUser` makes the page open on our entry rather than at the
    // top of the list. It is the name just written into
    // `RegisteredApplications`, so it only resolves because registration ran
    // first.
    let target = HSTRING::from(format!(
      "ms-settings:defaultapps?registeredAppUser={APP_NAME}"
    ));
    let operation = HSTRING::from("open");

    // ShellExecuteW hands the URI to a shell extension, and shell extensions
    // are COM objects. This runs on a `spawn_blocking` thread, which has no
    // apartment of its own, so give it one. An error means the thread already
    // had an apartment in another mode, and in that case it is not ours to
    // tear down.
    let com_status =
      unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let owns_com = com_status.is_ok();

    let result = unsafe {
      ShellExecuteW(
        None,
        PCWSTR(operation.as_ptr()),
        PCWSTR(target.as_ptr()),
        PCWSTR::null(),
        PCWSTR::null(),
        SW_SHOWNORMAL,
      )
    };

    if owns_com {
      unsafe { CoUninitialize() };
    }

    // ShellExecuteW reports success as a value above 32. Anything at or below
    // that is an error code wearing a handle's type.
    let code = result.0 as isize;
    if code <= 32 {
      return Err(format!(
        "Donut Browser is registered, but Windows Settings did not open (code {code}). Open Settings, then Apps, then Default apps, find Donut Browser and set it for HTTP and HTTPS."
      ));
    }

    Ok(())
  }

  /// Tell the shell that the association it has cached is stale.
  ///
  /// `SHChangeNotify` is the documented announcement for an association change,
  /// and the `WM_SETTINGCHANGE` broadcast is what the shell's own settings UI
  /// sends alongside it, so both go out.
  ///
  /// This used to hand-declare `SendMessageTimeoutA` with `lpdwResult` typed as
  /// `*mut u32` and pass it a `u32`. The real parameter is `PDWORD_PTR`, eight
  /// bytes on x64, so every call wrote four bytes past a stack slot. The result
  /// was a corrupted stack at the exact moment a user set Donut as their default
  /// browser, and the process died with nothing in the log. Go through the
  /// `windows` crate instead, which types the out-parameter correctly and cannot
  /// drift from the real ABI.
  fn notify_system_of_changes() {
    use windows::core::w;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    use windows::Win32::UI::WindowsAndMessaging::{
      SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    unsafe {
      SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);

      // The broadcast is best-effort: a hung top-level window elsewhere on the
      // desktop must not hold up the click that triggered this, hence the
      // timeout and SMTO_ABORTIFHUNG. `WM_SETTINGCHANGE`'s lParam string is
      // marshalled cross-process by the window manager, and this one is
      // 'static, so it stays valid for the whole call.
      let mut result: usize = 0;
      SendMessageTimeoutW(
        HWND_BROADCAST,
        WM_SETTINGCHANGE,
        WPARAM(0),
        LPARAM(w!("Software\\Classes").as_ptr() as isize),
        SMTO_ABORTIFHUNG,
        1000,
        Some(&mut result),
      );
    }
  }

  #[cfg(test)]
  mod registration_tests {
    use super::*;

    /// A scratch key that stands in for HKCU, so the test writes a real tree
    /// through the real code without touching the tree Windows actually reads.
    /// Deleted on the way out, including when an assertion fails.
    struct ScratchRoot {
      key: RegKey,
      path: String,
    }

    const SCRATCH_PARENT: &str = r"Software\DonutBrowserTests";

    impl ScratchRoot {
      fn new(name: &str) -> Self {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(r"{SCRATCH_PARENT}\{name}");
        let _ = hkcu.delete_subkey_all(&path);
        let (key, _) = hkcu.create_subkey(&path).expect("create the scratch root");
        Self { key, path }
      }

      fn value(&self, subkey: &str, name: &str) -> Option<String> {
        self
          .key
          .open_subkey(subkey)
          .ok()?
          .get_value::<String, _>(name)
          .ok()
      }
    }

    impl Drop for ScratchRoot {
      fn drop(&mut self) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(&self.path);
        // Take the shared parent too, so a test run leaves nothing at all in
        // the user's registry. `delete_subkey` refuses a key that still has
        // children, which is exactly the guard needed while tests run in
        // parallel: whoever finishes last removes it.
        let _ = hkcu.delete_subkey(SCRATCH_PARENT);
      }
    }

    const EXE: &str = r"C:\Program Files\Donut Browser\donutbrowser.exe";

    #[test]
    fn registration_writes_the_shape_the_shell_reads() {
      let root = ScratchRoot::new("registration");
      register_prog_id(&root.key, EXE).expect("register the ProgId");
      register_client(&root.key, EXE).expect("register the client");
      register_file_extensions(&root.key).expect("register the file types");
      register_application(&root.key).expect("register the application");

      // The bug that made the button do nothing: this pointed at the
      // application key instead of at its `Capabilities` subkey, so the shell
      // read no capabilities and never offered Donut as a browser. Every other
      // entry on a working machine ends in `Capabilities`.
      let registered = root
        .value(r"Software\RegisteredApplications", APP_NAME)
        .expect("RegisteredApplications entry");
      assert_eq!(registered, CAPABILITIES_KEY);
      assert!(
        registered.ends_with(r"\Capabilities"),
        "RegisteredApplications must name the Capabilities subkey, got {registered}"
      );
      assert!(
        root.key.open_subkey(&registered).is_ok(),
        "RegisteredApplications names {registered}, which does not exist"
      );

      // The second bug: `ApplicationName` sat one level above `Capabilities`,
      // where the shell does not look, so the entry had no name to show.
      assert_eq!(
        root.value(CAPABILITIES_KEY, "ApplicationName").as_deref(),
        Some(DISPLAY_NAME)
      );
      assert_eq!(
        root
          .value(CAPABILITIES_KEY, "ApplicationDescription")
          .as_deref(),
        Some(DESCRIPTION)
      );

      // Every scheme and file type the capability lists claim.
      for scheme in URL_SCHEMES {
        assert_eq!(
          root
            .value(&format!(r"{CAPABILITIES_KEY}\URLAssociations"), scheme)
            .as_deref(),
          Some(PROG_ID),
          "{scheme} is not claimed"
        );
      }
      for extension in FILE_EXTENSIONS {
        assert_eq!(
          root
            .value(&format!(r"{CAPABILITIES_KEY}\FileAssociations"), extension)
            .as_deref(),
          Some(PROG_ID),
          "{extension} is not claimed"
        );
      }

      // The rest of the StartMenuInternet entry.
      assert_eq!(root.value(CLIENT_KEY, "").as_deref(), Some(DISPLAY_NAME));
      assert_eq!(
        root.value(&format!(r"{CLIENT_KEY}\shell\open\command"), ""),
        Some(quoted(EXE))
      );
      assert!(root
        .key
        .open_subkey(format!(r"{CLIENT_KEY}\InstallInfo"))
        .is_ok());

      // The ProgId command has to carry `%1`. Without it the shell starts the
      // browser and never says which page to open.
      let prog_id_command = root
        .value(
          &format!(r"Software\Classes\{PROG_ID}\shell\open\command"),
          "",
        )
        .expect("ProgId command");
      assert_eq!(prog_id_command, format!(r#"{} "%1""#, quoted(EXE)));

      // The file types are offered, not seized. Taking the default value of
      // `Software\Classes\.html` is what the old code did, and that value
      // belongs to whatever the user chose.
      for extension in FILE_EXTENSIONS {
        assert_eq!(
          root
            .value(
              &format!(r"Software\Classes\{extension}\OpenWithProgids"),
              PROG_ID
            )
            .as_deref(),
          Some(""),
          "{extension} should offer the handler"
        );
        assert!(
          root
            .value(&format!(r"Software\Classes\{extension}"), "")
            .is_none(),
          "{extension} default value must be left alone"
        );
      }
    }

    #[test]
    fn the_association_an_older_build_took_is_given_back() {
      let root = ScratchRoot::new("legacy");

      // Recreate what the old code left behind: its own application key, and
      // the ProgId written straight into the association for one file type.
      let (legacy, _) = root
        .key
        .create_subkey(format!(r"{LEGACY_APP_KEY}\Capabilities\URLAssociations"))
        .expect("legacy key");
      legacy.set_value("http", &PROG_ID).expect("legacy claim");

      let (html, _) = root
        .key
        .create_subkey(r"Software\Classes\.html")
        .expect("html class");
      html.set_value("", &PROG_ID).expect("legacy association");

      // A file type the user pointed somewhere else. This one is not ours and
      // must survive untouched.
      let (htm, _) = root
        .key
        .create_subkey(r"Software\Classes\.htm")
        .expect("htm class");
      htm.set_value("", &"ChromeHTML").expect("user association");

      remove_legacy_registration(&root.key);

      assert!(
        root.key.open_subkey(LEGACY_APP_KEY).is_err(),
        "the superseded application key should be gone"
      );
      assert!(
        root.value(r"Software\Classes\.html", "").is_none(),
        "the association we took should have been released"
      );
      assert_eq!(
        root.value(r"Software\Classes\.htm", "").as_deref(),
        Some("ChromeHTML"),
        "a choice that is not ours must not be touched"
      );
    }
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use std::process::Command;

  const APP_DESKTOP_NAME: &str = "donutbrowser.desktop";

  pub fn is_default_browser() -> Result<bool, String> {
    // Check if xdg-mime is available
    if !is_xdg_mime_available() {
      return Err("xdg-mime utility not found. Please install xdg-utils package.".to_string());
    }

    let schemes = ["http", "https"];

    for scheme in schemes {
      let mime_type = format!("x-scheme-handler/{}", scheme);

      // Query the current default handler for this scheme
      let output = Command::new("xdg-mime")
        .args(["query", "default", &mime_type])
        .output()
        .map_err(|e| format!("Failed to query default handler for {}: {}", scheme, e))?;

      if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("xdg-mime query failed for {}: {}", scheme, stderr));
      }

      let current_handler = String::from_utf8_lossy(&output.stdout).trim().to_string();

      // Check if our app is the default handler
      if current_handler != APP_DESKTOP_NAME {
        return Ok(false);
      }
    }

    Ok(true)
  }

  pub fn set_as_default_browser() -> Result<(), String> {
    // Check if xdg-mime is available
    if !is_xdg_mime_available() {
      return Err("xdg-mime utility not found. Please install xdg-utils package.".to_string());
    }

    // Check if the desktop file exists in common locations
    if !check_desktop_file_exists() {
      return Err(format!(
        "Desktop file '{}' not found in standard locations. Please ensure the application is properly installed. You can manually set Donut Browser as the default browser in your system settings.",
        APP_DESKTOP_NAME
      ));
    }

    let schemes = ["http", "https"];
    let mut all_succeeded = true;
    let mut error_messages = Vec::new();

    for scheme in schemes {
      let mime_type = format!("x-scheme-handler/{}", scheme);

      // Set our app as the default handler for this scheme
      let output = Command::new("xdg-mime")
        .args(["default", APP_DESKTOP_NAME, &mime_type])
        .output()
        .map_err(|e| format!("Failed to set default handler for {}: {}", scheme, e))?;

      if !output.status.success() {
        all_succeeded = false;
        let stderr = String::from_utf8_lossy(&output.stderr);
        error_messages.push(format!("Failed to set default for {}: {}", scheme, stderr));
      }
    }

    if !all_succeeded {
      return Err(format!(
        "Some xdg-mime commands failed:\n{}\n\nYou may need to:\n1. Run with appropriate permissions\n2. Manually set the default browser in your desktop environment settings\n3. Restart your desktop session",
        error_messages.join("\n")
      ));
    }

    // Give the system a moment to process the changes
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify the changes took effect
    match is_default_browser() {
      Ok(true) => Ok(()),
      Ok(false) => {
        // This is the common case where commands succeed but verification fails
        Err(format!(
          "The xdg-mime commands completed successfully, but Donut Browser is not yet set as the default. This is common on some Linux distributions. Please try one of these options:\n\n1. Restart your desktop session and try again\n2. Log out and log back in\n3. Manually set Donut Browser as the default in your system settings:\n   - GNOME: Settings > Default Applications > Web\n   - KDE: System Settings > Applications > Default Applications > Web Browser\n   - XFCE: Settings > Preferred Applications > Web Browser\n   - Or run: xdg-settings set default-web-browser {}\n\nThe changes may take effect automatically after a desktop restart.",
          APP_DESKTOP_NAME
        ))
      }
      Err(e) => Err(format!(
        "Set as default completed, but verification failed: {}. The changes may still be in effect after restarting your desktop session.",
        e
      ))
    }
  }

  fn is_xdg_mime_available() -> bool {
    Command::new("which")
      .arg("xdg-mime")
      .output()
      .map(|output| output.status.success())
      .unwrap_or(false)
  }

  fn check_desktop_file_exists() -> bool {
    let desktop_locations = [
      "~/.local/share/applications/",
      "/usr/share/applications/",
      "/usr/local/share/applications/",
      "/var/lib/flatpak/exports/share/applications/",
      "~/.local/share/flatpak/exports/share/applications/",
    ];

    for location in &desktop_locations {
      let path = if location.starts_with('~') {
        if let Ok(home) = std::env::var("HOME") {
          location.replace('~', &home)
        } else {
          continue;
        }
      } else {
        location.to_string()
      };

      let full_path = format!("{}{}", path, APP_DESKTOP_NAME);
      if std::path::Path::new(&full_path).exists() {
        return true;
      }
    }

    false
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref DEFAULT_BROWSER: DefaultBrowser = DefaultBrowser::new();
}

#[command]
pub async fn is_default_browser() -> Result<bool, String> {
  let default_browser = DefaultBrowser::instance();
  default_browser.is_default_browser().await
}

#[command]
pub async fn set_as_default_browser() -> Result<SetDefaultOutcome, String> {
  let default_browser = DefaultBrowser::instance();
  default_browser.set_as_default_browser().await
}

#[cfg(test)]
mod tests {
  /// The type system now prevents the mistake behind the crash on Windows.
  /// `SendMessageTimeoutW` comes from the `windows` crate, and its
  /// out-parameter is typed `Option<*mut usize>`, so a four byte slot no longer
  /// compiles. That guarantee holds only while the call goes through the crate.
  /// A hand-written declaration would bring back the whole class of bug in a
  /// form no compiler and no lint can see, so refuse one here.
  ///
  /// This looks at the Windows module on every platform, because the module is
  /// compiled out everywhere else and would otherwise go unchecked on the
  /// runners that do most of the work.
  #[test]
  fn the_windows_module_declares_no_foreign_functions_by_hand() {
    const SOURCE: &str = include_str!("default_browser.rs");

    let start = SOURCE
      .find("mod windows {")
      .expect("the Windows module was renamed; update this guard");
    let end = SOURCE
      .find("mod linux {")
      .expect("the Linux module was renamed; update this guard");
    assert!(
      start < end,
      "the module order changed; update this guard so it still reads the Windows module"
    );

    assert!(
      !SOURCE[start..end].contains(r#"extern ""#),
      "The Windows module declares a foreign function by hand. Do not. A \
       hand-written declaration of SendMessageTimeoutA, with its out-parameter \
       typed *mut u32 instead of the real PDWORD_PTR, is what made Windows \
       write four bytes past a stack slot and kill the process every time a \
       user set Donut as their default browser. Take the binding from the \
       `windows` crate, which cannot drift from the real ABI, and add the \
       feature it needs to Cargo.toml."
    );
  }

  /// Show why the out-parameter has to be pointer sized.
  ///
  /// This does not try to reproduce the crash. Whether the four byte overrun is
  /// fatal depends on the frame the optimiser happens to build, so a crash test
  /// passes under one profile and fails under another. It measures the thing
  /// that is always true instead: the call writes eight bytes.
  #[cfg(target_os = "windows")]
  #[test]
  fn send_message_timeout_writes_a_pointer_sized_result() {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_NULL};

    /// A four byte slot with a marker behind it, laid out the way the old code
    /// laid out its `u32`. Eight bytes in total and eight byte aligned, so a
    /// pointer sized write lands entirely inside the struct. Nothing outside it
    /// is touched and the test is not itself undefined behaviour.
    #[repr(C, align(8))]
    struct Probe {
      result: u32,
      canary: u32,
    }

    const SENTINEL: u32 = 0xDEAD_BEEF;

    let mut probe = Probe {
      result: SENTINEL,
      canary: SENTINEL,
    };

    // The window handle is deliberately not a window. USER32 clears the
    // out-parameter before it looks at the target, so this measures the write
    // width without creating a window, without a message loop and without
    // sending anything to another process. The test is hermetic.
    unsafe {
      SendMessageTimeoutW(
        HWND(0xDEAD_0000_usize as *mut core::ffi::c_void),
        WM_NULL,
        WPARAM(0),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        50,
        Some(&mut probe as *mut Probe as *mut usize),
      );
    }

    assert_eq!(
      probe.result, 0,
      "SendMessageTimeoutW did not write the out-parameter at all, so this test \
       no longer measures anything. Check the call before trusting it."
    );
    assert_ne!(
      probe.canary, SENTINEL,
      "SendMessageTimeoutW wrote only four bytes. If Windows has really narrowed \
       lpdwResult to a DWORD then notify_system_of_changes may use a u32. Until \
       then the out-parameter stays pointer sized."
    );
  }
}
