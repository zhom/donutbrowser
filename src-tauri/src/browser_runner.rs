use crate::browser::ProxySettings;
use crate::cloud_auth::CLOUD_AUTH;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::events;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
use crate::wayfern_manager::{WayfernConfig, WayfernManager};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static PROFILE_LAUNCH_LOCKS: LazyLock<
  tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

/// How long a remote navigation waits for the page to settle.
///
/// A relayed round trip crosses two networks and the page load itself happens
/// on hardware in another country, so this is deliberately the same budget the
/// automation tools give a navigation rather than a loopback-sized one.
const REMOTE_NAVIGATE_TIMEOUT_SECS: u64 = 30;

async fn lock_profile_launch(profile_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
  let lock = {
    let mut locks = PROFILE_LAUNCH_LOCKS.lock().await;
    locks
      .entry(profile_id.to_string())
      .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
      .clone()
  };
  lock.lock_owned().await
}

pub struct BrowserRunner {
  pub profile_manager: &'static ProfileManager,
  pub downloaded_browsers_registry: &'static DownloadedBrowsersRegistry,
  auto_updater: &'static crate::auto_updater::AutoUpdater,
  wayfern_manager: &'static WayfernManager,
}

impl BrowserRunner {
  fn new() -> Self {
    Self {
      profile_manager: ProfileManager::instance(),
      downloaded_browsers_registry: DownloadedBrowsersRegistry::instance(),
      auto_updater: crate::auto_updater::AutoUpdater::instance(),
      wayfern_manager: WayfernManager::instance(),
    }
  }

  pub fn instance() -> &'static BrowserRunner {
    &BROWSER_RUNNER
  }

  pub fn get_binaries_dir(&self) -> PathBuf {
    crate::app_dirs::binaries_dir()
  }

  /// Resolve the DNS blocklist level to a cached file path plus whether that
  /// file should be treated as an allowlist. If a level is set but the cache
  /// is missing, fetches/compiles on demand (blocks until done).
  async fn resolve_blocklist_file(
    profile: &crate::profile::BrowserProfile,
  ) -> Result<(Option<String>, bool), String> {
    let Some(ref level_str) = profile.dns_blocklist else {
      return Ok((None, false));
    };
    let Some(level) = crate::dns_blocklist::BlocklistLevel::parse_level(level_str) else {
      return Ok((None, false));
    };
    if level == crate::dns_blocklist::BlocklistLevel::None {
      return Ok((None, false));
    }
    // Only the user's custom list can be an allowlist; the Hagezi tiers are
    // always block lists.
    let allowlist_mode = level == crate::dns_blocklist::BlocklistLevel::Custom
      && crate::dns_blocklist::CustomDnsConfig::load().allowlist_mode;
    let path = crate::dns_blocklist::BlocklistManager::ensure_cached(level)
      .await
      .map_err(|e| format!("Failed to fetch DNS blocklist: {e}"))?;
    Ok((Some(path.to_string_lossy().to_string()), allowlist_mode))
  }

  /// Refresh cloud proxy credentials if the profile uses a cloud or cloud-derived proxy,
  /// then resolve the proxy settings with profile-specific sid for sticky sessions.
  async fn resolve_proxy_with_refresh(
    &self,
    proxy_id: Option<&String>,
    profile_id: Option<&str>,
  ) -> Result<Option<ProxySettings>, String> {
    let proxy_id = match proxy_id {
      Some(id) => id,
      None => return Ok(None),
    };

    if PROXY_MANAGER.is_cloud_or_derived(proxy_id) {
      log::info!("Refreshing cloud proxy credentials before launch for proxy {proxy_id}");
      CLOUD_AUTH.sync_cloud_proxy().await;
    }
    // For cloud-derived proxies, inject profile-specific sid for sticky sessions
    if let Some(pid) = profile_id {
      if PROXY_MANAGER.is_cloud_or_derived(proxy_id) {
        return Ok(PROXY_MANAGER.resolve_proxy_for_profile(proxy_id, pid));
      }
    }
    Ok(PROXY_MANAGER.get_proxy_settings_by_id(proxy_id))
  }

  fn fire_launch_hook(profile: &BrowserProfile) {
    let Some(raw_url) = profile.launch_hook.as_deref() else {
      return;
    };
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
      return;
    }

    let parsed = match url::Url::parse(trimmed) {
      Ok(u) => u,
      Err(e) => {
        log::warn!(
          "Skipping launch hook for profile {} (ID: {}): invalid URL: {e}",
          profile.name,
          profile.id
        );
        return;
      }
    };

    if !matches!(parsed.scheme(), "http" | "https") {
      log::warn!(
        "Skipping launch hook for profile {} (ID: {}): URL must be http or https",
        profile.name,
        profile.id
      );
      return;
    }

    let url = parsed.to_string();
    let url_label = crate::log_redaction::url_label(&url);

    log::info!("Firing launch hook GET {url_label}");

    tokio::spawn(async move {
      let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
      {
        Ok(c) => c,
        Err(e) => {
          log::warn!(
            "Launch hook client build failed: {}",
            crate::log_redaction::text(&e.to_string())
          );
          return;
        }
      };

      match client.get(&url).send().await {
        Ok(resp) => {
          log::info!("Launch hook {url_label} returned status {}", resp.status());
        }
        Err(e) => {
          log::warn!(
            "Launch hook {url_label} failed: {}",
            crate::log_redaction::text(&e.to_string())
          );
        }
      }
    });
  }

  /// Resolve the upstream a launch will use.
  ///
  /// Deliberately does NOT fire the launch hook: that moved below the gate, so
  /// a launch the user blocks and then retries calls the user's webhook once
  /// rather than once per attempt.
  async fn resolve_launch_proxy(
    &self,
    profile: &BrowserProfile,
  ) -> Result<Option<ProxySettings>, String> {
    self
      .resolve_proxy_with_refresh(profile.proxy_id.as_ref(), Some(&profile.id.to_string()))
      .await
  }

  /// Get the executable path for a browser profile
  /// This is a common helper to eliminate code duplication across the codebase
  pub fn get_browser_executable_path(
    &self,
    profile: &BrowserProfile,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Create browser instance to get executable path
    let browser_type = crate::browser::BrowserType::from_str(&profile.browser)
      .map_err(|e| format!("Invalid browser type: {e}"))?;
    let browser = crate::browser::create_browser(browser_type);

    // Construct browser directory path: binaries/<browser>/<version>/
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(&profile.browser);
    browser_dir.push(&profile.version);

    // Get platform-specific executable path
    browser
      .get_executable_path(&browser_dir)
      .map_err(|e| format!("Failed to get executable path for {}: {e}", profile.browser).into())
  }

  async fn launch_browser_internal(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
    gate: &crate::launch_gate::FingerprintGate,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Handle Wayfern profiles using WayfernManager
    if profile.browser == "wayfern" {
      // Get or create wayfern config
      let mut wayfern_config = profile.wayfern_config.clone().unwrap_or_else(|| {
        log::info!(
          "No wayfern config found for profile {}, using default",
          profile.name
        );
        WayfernConfig::default()
      });

      // Always start a local proxy for Wayfern (for traffic monitoring and geoip support)
      let mut upstream_proxy = self
        .resolve_launch_proxy(profile)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      let geo_proxy_signature_settings = upstream_proxy.clone();

      struct XrayLaunchGuard {
        worker_id: Option<String>,
        profile_name: String,
      }
      impl Drop for XrayLaunchGuard {
        fn drop(&mut self) {
          let Some(worker_id) = self.worker_id.take() else {
            return;
          };
          log::warn!(
            "Launch failed after Xray-core start for profile {}; stopping worker",
            self.profile_name
          );
          if let Err(error) = crate::xray_worker_runner::stop_xray_worker_now(&worker_id) {
            log::warn!("Failed to stop Xray-core worker after failed launch: {error}");
          }
        }
      }
      let mut xray_launch_guard = XrayLaunchGuard {
        worker_id: None,
        profile_name: profile.name.clone(),
      };

      if upstream_proxy
        .as_ref()
        .is_some_and(|proxy| proxy.proxy_type.eq_ignore_ascii_case("vless"))
      {
        let vless_uri = upstream_proxy
          .as_ref()
          .and_then(|proxy| proxy.vless_uri.as_deref())
          .ok_or_else(|| crate::backend_error("VLESS_CONFIG_INVALID"))?;
        let worker =
          crate::xray_worker_runner::start_xray_worker(Some(&profile.id.to_string()), vless_uri)
            .await
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
              error.to_string().into()
            })?;
        log::info!(
          "Xray-core worker started for Wayfern profile on port {}",
          worker.local_port
        );
        xray_launch_guard.worker_id = Some(worker.id.clone());
        upstream_proxy = Some(worker.local_proxy_settings());
      }

      /// Stops a VPN worker this launch started, if the launch then fails.
      ///
      /// `created` is the whole point: `start_vpn_worker` reuses a live worker
      /// for the same VPN, so an unconditional stop would sever the tunnel a
      /// *different* profile is browsing through the moment this one is
      /// cancelled. The in-use check is a second belt for a worker adopted by a
      /// browser that started between the two points.
      struct VpnLaunchGuard {
        worker_id: Option<String>,
        vpn_id: String,
        created: bool,
        profile_name: String,
      }
      impl Drop for VpnLaunchGuard {
        fn drop(&mut self) {
          let Some(worker_id) = self.worker_id.take() else {
            return;
          };
          if !self.created {
            return;
          }
          log::warn!(
            "Launch failed after VPN worker start for profile {}; stopping worker",
            self.profile_name
          );
          let vpn_id = self.vpn_id.clone();
          tauri::async_runtime::spawn(async move {
            // Serialize against worker startup for the whole check-then-stop.
            // Without it another launch can adopt this worker between the
            // in-use check and the kill, and lose its tunnel a moment later.
            let _adopt_guard = crate::vpn_worker_runner::lock_vpn_starts().await;
            if crate::vpn_worker_runner::vpn_id_in_use_by_running_browser(&vpn_id) {
              log::info!("VPN {vpn_id} is still in use by a running browser; leaving it up");
              return;
            }
            if let Err(error) = crate::vpn_worker_runner::stop_vpn_worker(&worker_id).await {
              log::warn!("Failed to stop VPN worker after failed launch: {error}");
            }
          });
        }
      }
      let mut vpn_launch_guard: Option<VpnLaunchGuard> = None;

      // If profile has a VPN instead of proxy, start VPN worker and use it as upstream
      if upstream_proxy.is_none() {
        if let Some(ref vpn_id) = profile.vpn_id {
          match crate::vpn_worker_runner::start_vpn_worker_tracked(vpn_id).await {
            Ok(started) => {
              vpn_launch_guard = Some(VpnLaunchGuard {
                worker_id: Some(started.config.id.clone()),
                vpn_id: vpn_id.clone(),
                created: started.created,
                profile_name: profile.name.clone(),
              });
              if let Some(port) = started.config.local_port {
                upstream_proxy = Some(ProxySettings {
                  proxy_type: "socks5".to_string(),
                  host: "127.0.0.1".to_string(),
                  port,
                  username: None,
                  password: None,
                  vless_uri: None,
                });
                log::info!("VPN worker started for Wayfern profile on port {}", port);
              }
            }
            Err(e) => {
              return Err(crate::backend_error_with_detail("VPN_WORKER_START_FAILED", e).into());
            }
          }
        }
      }

      // The gate sits exactly here on purpose. By this line the upstream is
      // fully normalized across all three transports — VLESS and VPN are
      // authenticated loopback workers, a stored proxy is its resolved
      // settings — so one probe covers every profile. And it is still ahead of
      // the local proxy worker, the decrypted profile copy, the extension
      // unpack, and the browser process, so a blocked launch has nothing to
      // undo beyond the two workers whose guards are already armed above.
      //
      // Run concurrently with the blocklist compile so the added wall clock is
      // max(), not sum().
      let (blocklist, gate_result) = tokio::join!(
        Self::resolve_blocklist_file(profile),
        crate::launch_gate::enforce_fingerprint_gate(profile, upstream_proxy.as_ref(), gate),
      );
      gate_result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      let (blocklist_file, dns_allowlist_mode) = blocklist?;

      // Past the gate: this launch is really happening, so tell the user's
      // webhook exactly once.
      Self::fire_launch_hook(profile);

      log::info!(
        "Starting local proxy for Wayfern profile: {} (upstream: {})",
        profile.name,
        upstream_proxy
          .as_ref()
          .map(|p| format!("{}:{}", p.host, p.port))
          .unwrap_or_else(|| "DIRECT".to_string())
      );

      // Start the proxy and get local proxy settings
      // If proxy startup fails, DO NOT launch Wayfern - it requires local proxy
      let profile_id_str = profile.id.to_string();
      // Unique per-launch key: a shared constant here would let concurrent
      // launches overwrite each other's active_proxies entry, ending with one
      // browser's worker tracked under another browser's PID.
      let launch_placeholder_pid = crate::proxy_manager::next_launch_placeholder_pid();
      let local_proxy = PROXY_MANAGER
        .start_proxy(
          app_handle.clone(),
          upstream_proxy.as_ref(),
          launch_placeholder_pid,
          Some(&profile_id_str),
          profile.proxy_bypass_rules.clone(),
          blocklist_file,
          dns_allowlist_mode,
          // Wayfern (Chromium) uses a local SOCKS5 proxy so QUIC and WebRTC
          // UDP can be routed through it (via SOCKS5 UDP ASSOCIATE) without
          // leaking the real IP, rather than being forced direct as they
          // would be over an HTTP CONNECT proxy.
          "socks5",
        )
        .await
        .map_err(|e| {
          let error_msg = crate::wrap_backend_error(e, "Failed to start local proxy for Wayfern");
          log::error!("{}", error_msg);
          error_msg
        })?;

      // If any step below fails before the browser is up, the detached worker
      // must be stopped here: its config never gets a browser_pid, so neither
      // the GUI sweeps nor the worker's own watchdog would ever reap it — it
      // would survive until machine reboot.
      struct ProxyLaunchGuard {
        app_handle: tauri::AppHandle,
        routing_pid: u32,
        profile_name: String,
        armed: bool,
      }
      impl Drop for ProxyLaunchGuard {
        fn drop(&mut self) {
          if self.armed {
            log::warn!(
              "Launch failed after local proxy start for profile {}; stopping proxy worker",
              self.profile_name
            );
            let app_handle = self.app_handle.clone();
            let pid = self.routing_pid;
            tauri::async_runtime::spawn(async move {
              if let Err(e) = PROXY_MANAGER.stop_proxy(app_handle, pid).await {
                log::warn!("Failed to stop proxy worker after failed launch: {e}");
              }
            });
          }
        }
      }
      let mut proxy_launch_guard = ProxyLaunchGuard {
        app_handle: app_handle.clone(),
        routing_pid: launch_placeholder_pid,
        profile_name: profile.name.clone(),
        armed: true,
      };

      // Format proxy URL for wayfern - use SOCKS5 for the local proxy so
      // Chromium proxies UDP (QUIC/WebRTC), not just TCP.
      let proxy_url = format!("socks5://{}:{}", local_proxy.host, local_proxy.port);

      // Set proxy in wayfern config
      wayfern_config.proxy = Some(proxy_url);

      log::info!(
        "Configured local proxy for Wayfern: {:?}",
        wayfern_config.proxy
      );

      // Check if we need to generate a device for this launch.
      //
      // Two cases share the block: the user asked for a fresh device on every
      // launch, or the profile stores none at all. The second is how a clone
      // arrives here — cloning clears the fingerprint and the identity so the
      // clone gets an independent device instead of the browser's default —
      // and it also covers any profile that reached disk without one, which
      // used to launch on whatever device the browser drew for itself.
      //
      // A profile that ALREADY stores a device keeps it across a browser
      // upgrade: nothing here mints a replacement, and its stored payload is
      // what the launch applies. The one thing that does replace a stored
      // device is the user asking for it - `randomize_fingerprint_on_launch`,
      // tested immediately below - which is a deliberate per-profile setting
      // and not a consequence of the version.
      let mut updated_profile = profile.clone();
      let randomize_requested = wayfern_config.randomize_fingerprint_on_launch == Some(true);
      let needs_device = wayfern_config.fingerprint.is_none();
      if randomize_requested || needs_device {
        if needs_device && !randomize_requested {
          log::info!(
            "No stored device for Wayfern profile {}; generating one",
            profile.name
          );
        } else {
          log::info!(
            "Generating random fingerprint for Wayfern profile: {}",
            profile.name
          );
        }

        // Create a config copy without the existing fingerprint to force generation of a new one
        let mut config_for_generation = wayfern_config.clone();
        config_for_generation.fingerprint = None;

        // A failed generation fails the launch on purpose: continuing would
        // start the browser on whatever device it drew for itself, unmanaged
        // and unrecorded, while the UI still reports a successful launch. For
        // an anti-detect product a silently wrong device is worse than no
        // launch at all, because nothing tells the user to stop using it.
        //
        // Structured rather than prose, because the most common failure is the
        // browser refusing a generation once the account's hourly quota is
        // spent. That has to reach the user as an explanation; a raw CDP string
        // is not one, and the frontend only translates a coded error.
        let generated = self
          .wayfern_manager
          .generate_fingerprint_config(&app_handle, profile, &config_for_generation)
          .await
          .map_err(|e| {
            let detail = e.to_string();
            // BOTH refusal texts, because this path serves BOTH releases. 151
            // says "Fingerprint generation limit reached for this account.";
            // the shipped 150 browser says "Too many profiles are being
            // created." Matching only the 151 wording leaves a quota-blocked
            // 150 user staring at a raw CDP string, which is the exact defect
            // this mapping exists to remove.
            if detail.contains("generation limit reached") || detail.contains("Too many profiles") {
              crate::backend_error_with_detail("WAYFERN_GENERATION_LIMIT_REACHED", detail)
            } else {
              crate::backend_error_with_detail("WAYFERN_FINGERPRINT_GENERATION_FAILED", detail)
            }
          })?;

        let geolocation_applied = generated.geolocation_applied;

        log::info!(
          "New fingerprint generated, length: {} chars, identity: {:?}",
          generated.fingerprint.len(),
          generated.identity_id
        );

        // Update the config with the new fingerprint for launching
        wayfern_config.fingerprint = Some(generated.fingerprint.clone());
        wayfern_config.identity_id = generated.identity_id.clone();
        wayfern_config.identity_baseline = generated.identity_baseline.clone();

        // Save the updated fingerprint to the profile so it persists.
        let mut updated_wayfern_config = updated_profile.wayfern_config.clone().unwrap_or_default();
        updated_wayfern_config.fingerprint = Some(generated.fingerprint);
        updated_wayfern_config.identity_id = generated.identity_id;
        updated_wayfern_config.identity_baseline = generated.identity_baseline;
        // Preserve the randomize flag so it persists across launches
        updated_wayfern_config.randomize_fingerprint_on_launch =
          wayfern_config.randomize_fingerprint_on_launch;
        // Preserve the OS setting so it's used for future fingerprint generation
        if wayfern_config.os.is_some() {
          updated_wayfern_config.os = wayfern_config.os.clone();
        }
        // Record which routing this fresh fingerprint's geolocation was built
        // for (provenance only — nothing rewrites the fingerprint from it).
        // Only when geolocation actually applied; otherwise leave it unset so a
        // later on-demand match can tell the location was never resolved.
        updated_wayfern_config.geo_proxy_signature = if geolocation_applied {
          Some(crate::wayfern_manager::WayfernManager::geo_signature(
            geo_proxy_signature_settings.as_ref(),
            profile.vpn_id.as_deref(),
            wayfern_config.geoip.as_ref(),
          ))
        } else {
          None
        };
        updated_profile.wayfern_config = Some(updated_wayfern_config.clone());

        log::info!(
          "Updated profile wayfern_config with new fingerprint for profile: {}, fingerprint length: {}",
          profile.name,
          updated_wayfern_config.fingerprint.as_ref().map(|f| f.len()).unwrap_or(0)
        );
      }
      // A non-randomize profile keeps its configured fingerprint verbatim, even
      // when its proxy/VPN routing has changed since the fingerprint was built.
      // We deliberately do NOT silently rewrite its timezone/language to match
      // the new exit: that hid every real fingerprint-vs-exit mismatch (a US
      // fingerprint behind a German exit would be quietly relabelled German
      // before the launch-time consistency check could see it). The check now
      // surfaces the mismatch, and the user re-matches on demand via
      // `match_profile_fingerprint_to_exit`.

      // Create ephemeral dir for ephemeral or password-protected profiles
      if profile.password_protected {
        crate::profile::password::prepare_for_launch(profile)
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      } else if profile.ephemeral {
        crate::ephemeral_dirs::create_ephemeral_dir(&profile.id.to_string())
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      }

      // Launch Wayfern browser
      log::info!("Launching Wayfern for profile: {}", profile.name);

      // Get profile path for Wayfern
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(&updated_profile, &profiles_dir);

      // Profiles imported by builds before the layout fix have their content at
      // the user-data-dir root instead of under `Default/`, so the browser has
      // never seen a byte of it. Move it into place now, while the profile is
      // provably not running. Secrets stay unreadable — the source key was
      // never captured and cannot be recovered after the fact — but history,
      // bookmarks, extensions and site data come back.
      match crate::profile_import::repair_legacy_layout(&profile_data_path) {
        Ok(true) => log::info!(
          "Repaired legacy import layout for profile: {}",
          updated_profile.name
        ),
        Ok(false) => {}
        Err(e) => log::warn!(
          "Could not repair legacy import layout for {}: {e}",
          updated_profile.name
        ),
      }

      let profile_path_str = profile_data_path.to_string_lossy().to_string();

      // Install extensions if an extension group is assigned
      let mut extension_paths = Vec::new();
      if updated_profile.extension_group_id.is_some() {
        let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
        match mgr.install_extensions_for_profile(&updated_profile, &profile_data_path) {
          Ok(paths) => {
            if !paths.is_empty() {
              log::info!(
                "Prepared {} Chromium extensions for profile: {}",
                paths.len(),
                updated_profile.name
              );
            }
            extension_paths = paths;
          }
          Err(e) => {
            log::warn!("Failed to install extensions for Wayfern profile: {e}");
          }
        }
      }

      // Get proxy URL from config
      let proxy_url = wayfern_config.proxy.as_deref();

      let wayfern_result = self
        .wayfern_manager
        .launch_wayfern(
          &app_handle,
          &updated_profile,
          &profile_path_str,
          &wayfern_config,
          url.as_deref(),
          proxy_url,
          profile.ephemeral,
          &extension_paths,
          remote_debugging_port,
          headless,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          // A refused apply reports itself as a structured error so the dialog
          // can name the cause. Prefixing it would put English in front of the
          // JSON the frontend parses, and the whole thing would reach the user
          // as raw machine output.
          crate::wrap_backend_error(e, "Failed to launch Wayfern").into()
        })?;

      // Get the process ID from launch result
      let Some(process_id) = wayfern_result.processId.filter(|pid| *pid != 0) else {
        if let Err(error) = self.wayfern_manager.stop_wayfern(&wayfern_result.id).await {
          log::warn!("Failed to stop Wayfern after it omitted its process ID: {error}");
        }
        return Err(
          crate::backend_error_with_detail(
            "INTERNAL_ERROR",
            "Wayfern did not report a process identifier",
          )
          .into(),
        );
      };
      log::info!("Wayfern launched successfully with PID: {process_id}");

      if let Err(error) = PROXY_MANAGER.update_proxy_pid(launch_placeholder_pid, process_id) {
        if let Err(stop_error) = self.wayfern_manager.stop_wayfern(&wayfern_result.id).await {
          log::warn!("Failed to stop Wayfern after proxy PID mapping failed: {stop_error}");
        }
        return Err(crate::backend_error_with_detail("INTERNAL_ERROR", error).into());
      }
      proxy_launch_guard.routing_pid = process_id;
      log::info!(
        "Updated proxy PID mapping from launch placeholder {launch_placeholder_pid} to actual PID: {process_id}"
      );
      if !PROXY_MANAGER.set_browser_pid_for_profile(&updated_profile.id.to_string(), process_id) {
        if let Err(error) = self.wayfern_manager.stop_wayfern(&wayfern_result.id).await {
          log::warn!("Failed to stop Wayfern after proxy worker reassignment failed: {error}");
        }
        return Err(crate::backend_error("INTERNAL_ERROR").into());
      }
      if let Some(worker_id) = xray_launch_guard.worker_id.as_deref() {
        if !crate::xray_worker_runner::set_browser_pid(worker_id, process_id) {
          if let Err(error) = self.wayfern_manager.stop_wayfern(&wayfern_result.id).await {
            log::warn!("Failed to stop Wayfern after Xray worker reassignment failed: {error}");
          }
          return Err(crate::backend_error("XRAY_START_FAILED").into());
        }
      }

      // The browser and every detached routing worker now share one verified
      // process identity, so later profile-persistence failures must not tear
      // down a live route.
      proxy_launch_guard.armed = false;
      xray_launch_guard.worker_id = None;
      if let Some(guard) = vpn_launch_guard.as_mut() {
        guard.worker_id = None;
      }

      // The apply command echoes back the device the browser actually used,
      // which may differ from the stored one. Persist it so the next launch
      // starts from that value — saved below via
      // save_process_info(&updated_profile).
      if let Some(used_fp) = wayfern_result.used_fingerprint.clone() {
        let mut cfg = updated_profile.wayfern_config.clone().unwrap_or_default();
        let baseline_changed = wayfern_result.used_identity_baseline.is_some()
          && cfg.identity_baseline != wayfern_result.used_identity_baseline;
        if cfg.fingerprint.as_deref() != Some(used_fp.as_str()) || baseline_changed {
          log::info!(
            "Persisting applied fingerprint echoed by Wayfern for profile: {} (len {})",
            profile.name,
            used_fp.len()
          );
          cfg.fingerprint = Some(used_fp);
          // The baseline must move with the fingerprint it was computed
          // against, or the next launch diffs the two apart and invents
          // overrides the user never asked for.
          if let Some(baseline) = wayfern_result.used_identity_baseline.clone() {
            cfg.identity_baseline = Some(baseline);
          }
          updated_profile.wayfern_config = Some(cfg);
        }
      }

      // Update profile with the process info
      updated_profile.process_id = Some(process_id);
      updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

      // Save the updated profile
      log::info!(
        "Saving profile {} with wayfern_config fingerprint length: {}",
        updated_profile.name,
        updated_profile
          .wayfern_config
          .as_ref()
          .and_then(|c| c.fingerprint.as_ref())
          .map(|f| f.len())
          .unwrap_or(0)
      );
      self.save_process_info(&updated_profile)?;
      let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
        let _ = tm.rebuild_from_profiles(&self.profile_manager.list_profiles().unwrap_or_default());
      });
      log::info!(
        "Successfully saved profile with process info: {}",
        updated_profile.name
      );

      // Emit profiles-changed to trigger frontend to reload profiles from disk
      if let Err(e) = events::emit_empty("profiles-changed") {
        log::warn!("Warning: Failed to emit profiles-changed event: {e}");
      }

      log::info!(
        "Emitting profile events for successful Wayfern launch: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = events::emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      // Emit minimal running changed event to frontend
      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }

      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: updated_profile.process_id.is_some(),
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        log::info!(
          "Successfully emitted profile-running-changed event for Wayfern {}: running={}",
          updated_profile.name,
          payload.is_running
        );
      }

      return Ok(updated_profile);
    }

    Err(format!("Unsupported browser type: {}", profile.browser).into())
  }

  pub async fn open_url_in_existing_browser(
    &self,
    _app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: &str,
    _internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle Wayfern profiles using WayfernManager
    if profile.browser == "wayfern" {
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      // Check if the process is running
      match self
        .wayfern_manager
        .find_wayfern_by_profile(&profile_path_str)
        .await
      {
        Some(_wayfern_process) => {
          log::info!(
            "Opening URL in existing Wayfern process for profile: {} (ID: {})",
            profile.name,
            profile.id
          );

          // Use CDP to open URL in a new tab
          self
            .wayfern_manager
            .open_url_in_tab(&profile_path_str, url)
            .await?;
          return Ok(());
        }
        None => {
          return Err("Wayfern browser is not running".into());
        }
      }
    }

    Err(format!("Unsupported browser type: {}", profile.browser).into())
  }

  pub async fn launch_browser_with_debugging(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
    gate: &crate::launch_gate::FingerprintGate,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Wayfern starts (and PID-reconciles) its own local proxy
    // inside `launch_browser_internal`, so we hand it None here rather than
    // staging a second, orphaned proxy worker.
    self
      .launch_browser_internal(
        app_handle,
        profile,
        url,
        remote_debugging_port,
        headless,
        gate,
      )
      .await
  }

  pub async fn launch_or_open_url(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    internal_proxy_settings: Option<&ProxySettings>,
    gate: &crate::launch_gate::FingerprintGate,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    log::info!(
      "launch_or_open_url called for profile: {} (ID: {})",
      profile.name,
      profile.id
    );

    // Get the most up-to-date profile data
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles in launch_or_open_url: {e}"))?;
    let updated_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone());

    log::info!(
      "Checking browser status for profile: {} (ID: {})",
      updated_profile.name,
      updated_profile.id
    );

    // Check if browser is already running
    let is_running = self
      .check_browser_status(app_handle.clone(), &updated_profile)
      .await
      .map_err(|e| format!("Failed to check browser status: {e}"))?;

    // Get the updated profile again after status check (PID might have been updated)
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles after status check: {e}"))?;
    let final_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| updated_profile.clone());

    log::info!(
      "Browser status check: running={is_running}, URL requested={}, PID present={}",
      url.is_some(),
      final_profile.process_id.is_some()
    );

    if is_running {
      if let Some(url_ref) = url.as_ref() {
        log::info!(
          "Opening {} in existing browser",
          crate::log_redaction::url_label(url_ref)
        );

        match self
          .open_url_in_existing_browser(
            app_handle.clone(),
            &final_profile,
            url_ref,
            internal_proxy_settings,
          )
          .await
        {
          Ok(()) => {
            log::info!("Successfully opened URL in existing browser");
            Ok(final_profile)
          }
          Err(e) => {
            log::info!(
              "Failed to open URL in existing browser: {}",
              crate::log_redaction::text(&e.to_string())
            );
            Err(e)
          }
        }
      } else {
        log::info!("Browser is already running and no URL was requested");
        Ok(final_profile)
      }
    } else {
      log::info!("Launching new browser instance - browser not running");
      self
        .launch_browser_internal(app_handle.clone(), &final_profile, url, None, false, gate)
        .await
    }
  }

  fn save_process_info(
    &self,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use the regular save_profile method which handles the UUID structure
    self.profile_manager.save_profile(profile).map_err(|e| {
      let error_string = e.to_string();
      Box::new(std::io::Error::other(error_string)) as Box<dyn std::error::Error + Send + Sync>
    })
  }

  pub async fn check_browser_status(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    self
      .profile_manager
      .check_browser_status(app_handle, profile)
      .await
  }

  pub async fn kill_browser_process(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _profile_launch_guard = lock_profile_launch(&profile.id.to_string()).await;

    // "Stop this profile" has to mean the browser that is actually running, and
    // for a profile on the leased fleet that browser is not on this machine.
    // Without this, stopping reported success, killed nothing, and left the
    // session running to its two-hour cap — billing the user for every minute
    // and holding their profile lock the whole time.
    if self.stop_remote_session_for(&app_handle, profile).await? {
      return Ok(());
    }

    self
      .kill_browser_process_unlocked(app_handle, profile)
      .await
  }

  /// Stop this profile's fleet session, if it has one. Returns whether it did.
  ///
  /// Guarded on there being no local process so a locally running profile never
  /// pays for the lookup, exactly as the open-URL path is: the profile lock
  /// makes a local and a remote browser mutually exclusive.
  async fn stop_remote_session_for(
    &self,
    app_handle: &tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if profile.process_id.is_some() {
      return Ok(false);
    }
    let profile_id = profile.id.to_string();
    let Some(session_id) = crate::remote_handoff::running_session_for_profile(&profile_id) else {
      return Ok(false);
    };

    log::info!(
      "Stopping remote session {session_id} for profile {} ({profile_id})",
      profile.name
    );
    crate::remote_session::end_remote_session(&session_id)
      .await
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        // Surfaced rather than swallowed. The backend refuses to retire a
        // session it could not stop on the fleet, so a failure here means the
        // browser is STILL RUNNING; reporting success would tell the user their
        // profile is free when a host is still writing to it.
        log::warn!("Failed to stop remote session {session_id}: {e}");
        e.to_error_json().into()
      })?;

    // The session is down and its work is in cloud storage. This is what puts
    // the profile into "pending sync" and starts the pull, so the user is not
    // handed back a profile directory that predates the session they just ran.
    //
    // The session's own profile lock is released by the backend when it retires
    // the row; nothing is released from here, because this client never held it.
    crate::remote_session::note_session_stopped(app_handle, &session_id);
    Ok(true)
  }

  async fn kill_browser_process_unlocked(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle Wayfern profiles using WayfernManager
    if profile.browser == "wayfern" {
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      log::info!(
        "Attempting to kill Wayfern process for profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      // Stop the proxy associated with this profile first
      let profile_id_str = profile.id.to_string();
      if let Err(e) = PROXY_MANAGER
        .stop_proxy_by_profile_id(app_handle.clone(), &profile_id_str)
        .await
      {
        log::warn!(
          "Warning: Failed to stop proxy for profile {}: {e}",
          profile_id_str
        );
      }
      if let Err(error) =
        crate::xray_worker_runner::stop_xray_worker_by_profile_id(&profile_id_str).await
      {
        log::warn!(
          "Warning: Failed to stop Xray-core worker for profile {}: {error}",
          profile_id_str
        );
      }

      let mut process_actually_stopped = false;
      match self
        .wayfern_manager
        .find_wayfern_by_profile(&profile_path_str)
        .await
      {
        Some(wayfern_process) => {
          log::info!(
            "Found Wayfern process: {} (PID: {:?})",
            wayfern_process.id,
            wayfern_process.processId
          );

          match self.wayfern_manager.stop_wayfern(&wayfern_process.id).await {
            Ok(_) => {
              if let Some(pid) = wayfern_process.processId {
                // Verify the process actually died by checking after a short delay
                use tokio::time::{sleep, Duration};
                sleep(Duration::from_millis(500)).await;

                use sysinfo::{Pid, System};
                let system = System::new_all();
                process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();

                if process_actually_stopped {
                  log::info!(
                    "Successfully stopped Wayfern process: {} (PID: {:?}) - verified process is dead",
                    wayfern_process.id,
                    pid
                  );
                } else {
                  log::warn!(
                    "Wayfern stop command returned success but process {} (PID: {:?}) is still running - forcing kill",
                    wayfern_process.id,
                    pid
                  );
                  // Force kill the process
                  #[cfg(target_os = "macos")]
                  {
                    use crate::platform_browser;
                    if let Err(e) = platform_browser::macos::kill_browser_process_impl(
                      pid,
                      Some(&profile_path_str),
                    )
                    .await
                    {
                      log::error!("Failed to force kill Wayfern process {}: {}", pid, e);
                    } else {
                      sleep(Duration::from_millis(500)).await;
                      let system = System::new_all();
                      process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                      if process_actually_stopped {
                        log::info!(
                          "Successfully force killed Wayfern process {} (PID: {:?})",
                          wayfern_process.id,
                          pid
                        );
                      }
                    }
                  }
                  #[cfg(target_os = "linux")]
                  {
                    use crate::platform_browser;
                    if let Err(e) = platform_browser::linux::kill_browser_process_impl(
                      pid,
                      Some(&profile_path_str),
                    )
                    .await
                    {
                      log::error!("Failed to force kill Wayfern process {}: {}", pid, e);
                    } else {
                      sleep(Duration::from_millis(500)).await;
                      let system = System::new_all();
                      process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                      if process_actually_stopped {
                        log::info!(
                          "Successfully force killed Wayfern process {} (PID: {:?})",
                          wayfern_process.id,
                          pid
                        );
                      }
                    }
                  }
                  #[cfg(target_os = "windows")]
                  {
                    use crate::platform_browser;
                    if let Err(e) = platform_browser::windows::kill_browser_process_impl(pid).await
                    {
                      log::error!("Failed to force kill Wayfern process {}: {}", pid, e);
                    } else {
                      sleep(Duration::from_millis(500)).await;
                      let system = System::new_all();
                      process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                      if process_actually_stopped {
                        log::info!(
                          "Successfully force killed Wayfern process {} (PID: {:?})",
                          wayfern_process.id,
                          pid
                        );
                      }
                    }
                  }
                }
              } else {
                process_actually_stopped = true;
              }
            }
            Err(e) => {
              log::error!(
                "Error stopping Wayfern process {}: {}",
                wayfern_process.id,
                e
              );
              // Try to force kill if we have a PID
              if let Some(pid) = wayfern_process.processId {
                log::info!(
                  "Attempting force kill after stop_wayfern error for PID: {}",
                  pid
                );
                #[cfg(target_os = "macos")]
                {
                  use crate::platform_browser;
                  if let Err(kill_err) =
                    platform_browser::macos::kill_browser_process_impl(pid, Some(&profile_path_str))
                      .await
                  {
                    log::error!("Failed to force kill Wayfern process {}: {}", pid, kill_err);
                  } else {
                    use tokio::time::{sleep, Duration};
                    sleep(Duration::from_millis(500)).await;
                    use sysinfo::{Pid, System};
                    let system = System::new_all();
                    process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                  }
                }
                #[cfg(target_os = "linux")]
                {
                  use crate::platform_browser;
                  if let Err(kill_err) =
                    platform_browser::linux::kill_browser_process_impl(pid, Some(&profile_path_str))
                      .await
                  {
                    log::error!("Failed to force kill Wayfern process {}: {}", pid, kill_err);
                  } else {
                    use tokio::time::{sleep, Duration};
                    sleep(Duration::from_millis(500)).await;
                    use sysinfo::{Pid, System};
                    let system = System::new_all();
                    process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                  }
                }
                #[cfg(target_os = "windows")]
                {
                  use crate::platform_browser;
                  if let Err(kill_err) =
                    platform_browser::windows::kill_browser_process_impl(pid).await
                  {
                    log::error!("Failed to force kill Wayfern process {}: {}", pid, kill_err);
                  } else {
                    use tokio::time::{sleep, Duration};
                    sleep(Duration::from_millis(500)).await;
                    use sysinfo::{Pid, System};
                    let system = System::new_all();
                    process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                  }
                }
              }
            }
          }
        }
        None => {
          log::info!(
            "No running Wayfern process found for profile: {} (ID: {})",
            profile.name,
            profile.id
          );
          process_actually_stopped = true;
        }
      }

      // If process wasn't confirmed stopped, return an error
      if !process_actually_stopped {
        log::error!(
          "Failed to stop Wayfern process for profile: {} (ID: {}) - process may still be running",
          profile.name,
          profile.id
        );
        return Err(
          format!(
            "Failed to stop Wayfern process for profile {} - process may still be running",
            profile.name
          )
          .into(),
        );
      }

      // Clear the process ID from the profile and save immediately so that
      // subsequent calls to update_profile_version (which re-reads from disk)
      // see the cleared process_id.
      let mut updated_profile = profile.clone();
      updated_profile.process_id = None;
      self
        .save_process_info(&updated_profile)
        .map_err(|e| format!("Failed to update profile: {e}"))?;

      // Check for pending updates and apply them
      if let Ok(Some(pending_update)) = self
        .auto_updater
        .get_pending_update(&profile.browser, &profile.version)
      {
        log::info!(
          "Found pending update for Wayfern profile {}: {} -> {}",
          profile.name,
          profile.version,
          pending_update.new_version
        );

        match self.profile_manager.update_profile_version(
          &app_handle,
          &profile.id.to_string(),
          &pending_update.new_version,
        ) {
          Ok(updated_profile_after_update) => {
            log::info!(
              "Successfully updated Wayfern profile {} from version {} to {}",
              profile.name,
              profile.version,
              pending_update.new_version
            );
            updated_profile = updated_profile_after_update;

            if let Err(e) = self
              .auto_updater
              .dismiss_update_notification(&pending_update.id)
            {
              log::warn!("Warning: Failed to dismiss pending update notification: {e}");
            }
          }
          Err(e) => {
            log::error!(
              "Failed to apply pending update for Wayfern profile {}: {}",
              profile.name,
              e
            );
          }
        }
      }

      // If no pending update was applied, check if a newer installed version exists
      if updated_profile.version == profile.version {
        if let Some(p) = self
          .auto_updater
          .update_profile_to_latest_installed(&app_handle, &updated_profile)
        {
          updated_profile = p;
        }
      }

      log::info!(
        "Emitting profile events for successful Wayfern kill: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = events::emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      // Emit minimal running changed event
      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: false,
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        log::info!(
          "Successfully emitted profile-running-changed event for Wayfern {}: running={}",
          updated_profile.name,
          payload.is_running
        );
      }

      if profile.password_protected {
        // Await the re-encryption so the queued sync (released later by
        // `mark_profile_stopped` in `kill_browser`) sees fresh ciphertext on
        // disk instead of the previous snapshot.
        crate::profile::password::complete_after_quit_and_wait(profile).await;
      } else if profile.ephemeral {
        let id = profile.id.to_string();
        crate::ephemeral_dirs::remove_ephemeral_dir(&id);
        // The per-domain traffic tracker writes to the cache dir on real disk
        // regardless of where the profile itself lives, so an "in memory only"
        // session still left a full record of everywhere it connected.
        crate::traffic_stats::delete_traffic_stats(&id);
      } else if profile.clear_on_close {
        // Awaited for the same reason as re-encryption above: a queued sync
        // must see the cleared dir, not the pre-clear snapshot.
        crate::profile::clear_on_close::clear_profile_browsing_data(profile).await;
      }

      // The browser held these open for the life of the process; nothing reads
      // them once it has exited, and they are plaintext extension code sitting
      // on real disk even for an ephemeral profile.
      crate::extension_manager::ExtensionManager::cleanup_unpacked_for_profile(
        &profile.id.to_string(),
      );

      log::info!(
        "Wayfern process cleanup completed for profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      // Consolidate browser versions after stopping a browser
      if let Ok(consolidated) = self
        .downloaded_browsers_registry
        .consolidate_browser_versions(&app_handle)
      {
        if !consolidated.is_empty() {
          log::info!("Post-stop version consolidation results:");
          for action in &consolidated {
            log::info!("  {action}");
          }
        }
      }

      return Ok(());
    }

    Err(
      format!(
        "Unsupported browser '{}' for profile '{}' — only Wayfern is supported",
        profile.browser, profile.name
      )
      .into(),
    )
  }

  pub async fn open_url_with_profile(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: String,
    url: String,
    gate: crate::launch_gate::FingerprintGate,
  ) -> Result<(), String> {
    // Get the profile by name
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let profile = profiles
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| format!("Profile '{profile_id}' not found"))?;
    let _profile_launch_guard = lock_profile_launch(&profile.id.to_string()).await;

    // A profile already open on the leased fleet is driven, not launched. This
    // sits above the cross-OS guard on purpose: a Windows profile cannot run on
    // this Mac, which is the whole reason it is running remotely, and refusing
    // to point it at a URL for that reason would make the remote session
    // unusable from the one endpoint that exists to use it.
    //
    // Guarded on there being no local process, so a profile running here never
    // pays for the lookup: a local launch records a pid, and the profile lock
    // keeps a local and a remote session mutually exclusive.
    if profile.process_id.is_none() {
      if let Ok(target) = crate::cdp_target::resolve(&profile).await {
        if target.is_remote() {
          log::info!("Opening URL through {}", target.describe());
          return crate::cdp_target::navigate(&target, &url, REMOTE_NAVIGATE_TIMEOUT_SECS)
            .await
            .map_err(|e| {
              log::warn!("Failed to open a URL on the remote browser: {e}");
              format!("Failed to open URL with profile: {e}")
            });
        }
      }
    }

    if profile.is_cross_os() {
      return Err(format!(
        "Cannot open URL with profile '{}': this profile was created on {} and cannot be used on a different operating system",
        profile.name,
        profile.host_os.as_deref().unwrap_or("another OS"),
      ));
    }

    // Past this point a local browser is about to be launched, and until now
    // this was the ONE launch path that took neither the profile lock nor any
    // notice of the fleet. A remote session whose state could not be read (a
    // dropped event stream plus an unreachable backend) fell straight through
    // to a local launch on a profile a host was writing to.
    crate::remote_handoff::ensure_local_launch_allowed(&profile.id.to_string())?;
    let acquired_team_lock = crate::team_lock::acquire_team_lock_if_needed(&profile).await?;

    log::info!("Opening URL with selected profile");

    // Use launch_or_open_url which handles both launching new instances and opening in existing ones
    if let Err(e) = self
      .launch_or_open_url(app_handle, &profile, Some(url.clone()), None, &gate)
      .await
    {
      log::info!(
        "Failed to open URL with selected profile: {}",
        crate::log_redaction::text(&e.to_string())
      );
      // This path takes the team lock too, and a blocked launch never records a
      // process_id for the status sweep to release it from.
      unwind_launch(&profile, acquired_team_lock).await;
      // Pass structured errors through untouched: the gate's block carries the
      // mismatch detail the dialog renders, and wrapping it in English would
      // reach the user as raw JSON.
      return Err(crate::wrap_backend_error(
        e,
        "Failed to open URL with profile",
      ));
    }

    log::info!("Successfully opened URL with selected profile");
    Ok(())
  }
}

#[tauri::command]
pub async fn launch_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
  consent_token: Option<String>,
) -> Result<BrowserProfile, String> {
  let options = LaunchOptions {
    gate: match consent_token {
      Some(token) => crate::launch_gate::FingerprintGate::Consented(token),
      None => crate::launch_gate::FingerprintGate::Enforce,
    },
    ..Default::default()
  };
  launch_browser_profile_impl(app_handle, profile, url, options).await
}

/// How one launch should behave.
///
/// A struct rather than four trailing positional arguments: `headless` and
/// `force_new` are already passed adjacently as bare booleans, so a fifth would
/// compile everywhere while silently inverting behavior wherever the order was
/// got wrong.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
  pub remote_debugging_port: Option<u16>,
  pub headless: bool,
  pub force_new: bool,
  pub gate: crate::launch_gate::FingerprintGate,
}

impl LaunchOptions {
  /// Automation defaults: report, never block, never probe. A headless client
  /// has no dialog to answer and cannot regenerate its fingerprint mid-run, so
  /// a hard failure would turn a warning into an outage for a whole fleet.
  pub fn automation(remote_debugging_port: Option<u16>, headless: bool) -> Self {
    Self {
      remote_debugging_port,
      headless,
      force_new: true,
      gate: crate::launch_gate::FingerprintGate::Advisory,
    }
  }
}

/// Release the team lock a launch attempt took before it failed.
///
/// Until the gate existed, failing here was rare enough that leaking was merely
/// untidy. Cancelling a blocked launch is now an ordinary outcome, and the lock
/// renews itself on a 30s heartbeat while only ever being released via a stored
/// `process_id` — which a launch that never spawned does not have. So a leak
/// leaves the profile reading as locked to the whole team until the app quits.
///
/// `acquired` is threaded from `acquire_team_lock_if_needed` so this releases
/// only what this call took, never a lock a REST handler up the stack owns.
///
/// Several of these error paths are reachable while a browser for the profile
/// is genuinely still running — `PROFILE_RUNNING`, or a failure to open a URL
/// in an existing window. That browser owns the lock and the running mark, so
/// releasing either would strand it: the team would see the profile as free
/// while someone is typing in it, and `mark_profile_stopped` would queue a sync
/// of a profile directory being written to. Hence the liveness check.
async fn unwind_launch(profile: &BrowserProfile, acquired_team_lock: bool) {
  if browser_is_running_for(&profile.id.to_string()) {
    log::debug!(
      "Not unwinding launch state for {}: a browser is still running for it",
      profile.name
    );
    return;
  }

  if acquired_team_lock {
    crate::team_lock::release_team_lock_if_needed(profile).await;
  }
  // Otherwise this mark sticks for the rest of the session and silently defers
  // every sync of the profile. Safe here precisely because nothing is running.
  if let Some(scheduler) = crate::sync::get_global_scheduler() {
    scheduler
      .mark_profile_stopped(&profile.id.to_string())
      .await;
  }
}

/// Whether a live browser process is recorded for this profile right now.
/// Re-read from disk: the caller's copy predates the launch attempt.
fn browser_is_running_for(profile_id: &str) -> bool {
  BrowserRunner::instance()
    .profile_manager
    .list_profiles()
    .ok()
    .and_then(|profiles| {
      profiles
        .into_iter()
        .find(|p| p.id.to_string() == profile_id)
        .map(|p| {
          p.process_id
            .is_some_and(crate::proxy_storage::is_process_running)
        })
    })
    .unwrap_or(false)
}

pub async fn launch_browser_profile_impl(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
  options: LaunchOptions,
) -> Result<BrowserProfile, String> {
  let LaunchOptions {
    remote_debugging_port,
    headless,
    force_new,
    gate,
  } = options;
  log::info!(
    "Launch request received for profile: {} (ID: {})",
    profile.name,
    profile.id
  );
  let _profile_launch_guard = lock_profile_launch(&profile.id.to_string()).await;

  if profile.is_cross_os() {
    return Err(format!(
      "Cannot launch profile '{}': this profile was created on {} and cannot be launched on a different operating system",
      profile.name,
      profile.host_os.as_deref().unwrap_or("another OS"),
    ));
  }

  // Refuse a launch that would run over work a remote session has not handed
  // back yet. Checked before the profile lock because it answers without a
  // round trip and because it stays true after the session's lock is released:
  // the lock protects the browser, this protects the bytes it wrote.
  crate::remote_handoff::ensure_local_launch_allowed(&profile.id.to_string())?;

  // Team lock check: if profile is sync-enabled and user is on a team, acquire lock
  let acquired_team_lock = crate::team_lock::acquire_team_lock_if_needed(&profile).await?;

  // Notify sync scheduler that profile is now running and queue sync for when it stops
  if let Some(scheduler) = crate::sync::get_global_scheduler() {
    let pid = profile.id.to_string();
    scheduler.mark_profile_running(&pid).await;
    if profile.is_sync_enabled() {
      scheduler.queue_profile_sync(pid).await;
    }
  }

  let browser_runner = BrowserRunner::instance();

  // Resolve the most up-to-date profile from disk by ID to avoid using stale proxy_id/browser state
  let profile_for_launch = match browser_runner
    .profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))
  {
    Ok(profiles) => profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone()),
    Err(e) => {
      unwind_launch(&profile, acquired_team_lock).await;
      return Err(e);
    }
  };

  log::info!(
    "Resolved profile for launch: {} (ID: {})",
    profile_for_launch.name,
    profile_for_launch.id
  );

  log::info!(
    "Starting browser launch for profile: {} (ID: {})",
    profile_for_launch.name,
    profile_for_launch.id
  );

  if force_new {
    let already_running = match browser_runner
      .check_browser_status(app_handle.clone(), &profile_for_launch)
      .await
    {
      Ok(running) => running,
      Err(error) => {
        unwind_launch(&profile, acquired_team_lock).await;
        return Err(crate::wrap_backend_error(
          error,
          "Failed to check browser status before launch",
        ));
      }
    };
    if already_running {
      unwind_launch(&profile, acquired_team_lock).await;
      return Err(crate::backend_error("PROFILE_RUNNING"));
    }
  }

  // Launch browser or open URL in existing instance. Wayfern starts its
  // own local proxy inside `launch_browser_internal`; other browser types
  // are rejected there, so no proxy needs to be staged here.
  //
  // `force_new` callers (API/MCP) always start a fresh instance with the
  // requested debug port and headless mode, bypassing the "open URL in the
  // existing window" path which would otherwise ignore both.
  let launch_result = if force_new {
    browser_runner
      .launch_browser_with_debugging(
        app_handle.clone(),
        &profile_for_launch,
        url,
        remote_debugging_port,
        headless,
        &gate,
      )
      .await
  } else {
    browser_runner
      .launch_or_open_url(app_handle.clone(), &profile_for_launch, url, None, &gate)
      .await
  };
  let updated_profile = match launch_result {
    Ok(updated) => updated,
    Err(e) => {
      log::info!(
        "Browser launch failed for profile: {}, error: {}",
        profile_for_launch.name,
        e
      );

      // Emit a failure event to clear loading states in the frontend
      #[derive(serde::Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      let payload = RunningChangedPayload {
        id: profile_for_launch.id.to_string(),
        is_running: false,
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      unwind_launch(&profile, acquired_team_lock).await;

      // Check if this is an architecture compatibility issue
      if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
        if io_error.kind() == std::io::ErrorKind::Other
          && io_error.to_string().contains("Exec format error")
        {
          return Err(format!("Failed to launch browser: Executable format error. This browser version is not compatible with your system architecture ({}). Please try a different browser or version that supports your platform.", std::env::consts::ARCH));
        }
      }
      return Err(crate::wrap_backend_error(
        e,
        "Failed to launch browser or open URL",
      ));
    }
  };

  log::info!(
    "Browser launch completed for profile: {} (ID: {})",
    updated_profile.name,
    updated_profile.id
  );

  // The proxy PID mapping was already reconciled inside launch_browser_internal
  // (placeholder → real browser PID); nothing is ever keyed by a constant here.

  Ok(updated_profile)
}

#[tauri::command]
pub fn check_browser_exists(browser_str: String, version: String) -> bool {
  // This is an alias for is_browser_downloaded to provide clearer semantics for auto-updates
  let runner = BrowserRunner::instance();
  runner
    .downloaded_browsers_registry
    .is_browser_downloaded(&browser_str, &version)
}

#[tauri::command]
pub async fn kill_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<(), String> {
  log::info!(
    "Kill request received for profile: {} (ID: {})",
    profile.name,
    profile.id
  );
  let browser_runner = BrowserRunner::instance();

  match browser_runner
    .kill_browser_process(app_handle.clone(), &profile)
    .await
  {
    Ok(()) => {
      log::info!(
        "Successfully killed browser profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      // Release team lock if applicable
      crate::team_lock::release_team_lock_if_needed(&profile).await;

      // Notify sync scheduler that profile stopped (sync was queued at launch)
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        scheduler
          .mark_profile_stopped(&profile.id.to_string())
          .await;
      }

      // Auto-update non-running profiles and cleanup unused binaries
      let browser_for_update = profile.browser.clone();
      let app_handle_for_update = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        let registry = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
        let mut versions = registry.get_downloaded_versions(&browser_for_update);
        if !versions.is_empty() {
          versions.sort_by(|a, b| crate::api_client::compare_versions(b, a));
          let latest_version = &versions[0];

          let auto_updater = crate::auto_updater::AutoUpdater::instance();
          match auto_updater
            .auto_update_profile_versions(
              &app_handle_for_update,
              &browser_for_update,
              latest_version,
            )
            .await
          {
            Ok(updated) => {
              if !updated.is_empty() {
                log::info!(
                  "Auto-updated {} profiles after stop: {:?}",
                  updated.len(),
                  updated
                );
              }
            }
            Err(e) => {
              log::error!("Failed to auto-update profile versions after stop: {e}");
            }
          }
        }

        match registry.cleanup_unused_binaries() {
          Ok(cleaned) => {
            if !cleaned.is_empty() {
              log::info!("Cleaned up unused binaries after stop: {:?}", cleaned);
            }
          }
          Err(e) => {
            log::error!("Failed to cleanup unused binaries after stop: {e}");
          }
        }
      });

      Ok(())
    }
    Err(e) => {
      log::info!("Failed to kill browser profile {}: {}", profile.name, e);

      // Emit a failure event to clear loading states in the frontend
      #[derive(serde::Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      // On kill failure, we assume the process is still running
      let payload = RunningChangedPayload {
        id: profile.id.to_string(),
        is_running: true,
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      Err(format!("Failed to kill browser: {e}"))
    }
  }
}

#[tauri::command]
pub async fn open_url_with_profile(
  app_handle: tauri::AppHandle,
  profile_id: String,
  url: String,
  consent_token: Option<String>,
) -> Result<(), String> {
  let browser_runner = BrowserRunner::instance();
  let gate = match consent_token {
    Some(token) => crate::launch_gate::FingerprintGate::Consented(token),
    None => crate::launch_gate::FingerprintGate::Enforce,
  };
  browser_runner
    .open_url_with_profile(app_handle, profile_id, url, gate)
    .await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn profile_launch_lock_serializes_only_the_same_profile() {
    let profile = format!("launch-lock-{}", uuid::Uuid::new_v4());
    let other_profile = format!("launch-lock-{}", uuid::Uuid::new_v4());
    let first = lock_profile_launch(&profile).await;

    assert!(tokio::time::timeout(
      Duration::from_millis(100),
      lock_profile_launch(&other_profile)
    )
    .await
    .is_ok());
    assert!(
      tokio::time::timeout(Duration::from_millis(100), lock_profile_launch(&profile))
        .await
        .is_err()
    );

    drop(first);
    assert!(
      tokio::time::timeout(Duration::from_millis(100), lock_profile_launch(&profile))
        .await
        .is_ok()
    );
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_RUNNER: BrowserRunner = BrowserRunner::new();
}
