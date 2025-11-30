use clap::{Arg, Command};
use donutbrowser_lib::proxy_runner::{
  start_proxy_process_with_profile, stop_all_proxy_processes, stop_proxy_process,
};
use donutbrowser_lib::proxy_server::run_proxy_server;
use donutbrowser_lib::proxy_storage::get_proxy_config;
use std::process;

fn set_high_priority() {
  #[cfg(unix)]
  {
    unsafe {
      // Set high priority (negative nice value = higher priority)
      // -10 is a reasonably high priority without being too aggressive
      // This may fail without elevated privileges, which is fine
      let result = libc::setpriority(libc::PRIO_PROCESS, 0, -10);
      if result == 0 {
        log::info!("Set process priority to -10 (high priority)");
      } else {
        // Try a less aggressive priority if -10 fails
        let result = libc::setpriority(libc::PRIO_PROCESS, 0, -5);
        if result == 0 {
          log::info!("Set process priority to -5 (above normal)");
        }
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    // Lower OOM score so this process is less likely to be killed under memory pressure
    // Valid range is -1000 to 1000, lower = less likely to be killed
    // -500 is a reasonable value that makes us less likely to be killed
    if let Err(e) = std::fs::write("/proc/self/oom_score_adj", "-500") {
      log::debug!("Could not set OOM score adjustment: {}", e);
    } else {
      log::info!("Set OOM score adjustment to -500");
    }
  }

  #[cfg(windows)]
  {
    use windows::Win32::System::Threading::{
      GetCurrentProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
    };

    unsafe {
      let process = GetCurrentProcess();
      if SetPriorityClass(process, ABOVE_NORMAL_PRIORITY_CLASS).is_ok() {
        log::info!("Set process priority to ABOVE_NORMAL_PRIORITY_CLASS");
      } else {
        log::debug!("Could not set process priority class");
      }
    }
  }
}

fn build_proxy_url(
  proxy_type: &str,
  host: &str,
  port: u16,
  username: Option<&str>,
  password: Option<&str>,
) -> String {
  let mut url = format!("{}://", proxy_type.to_lowercase());

  if let (Some(user), Some(pass)) = (username, password) {
    let encoded_user = urlencoding::encode(user);
    let encoded_pass = urlencoding::encode(pass);
    url.push_str(&format!("{}:{}@", encoded_user, encoded_pass));
  } else if let Some(user) = username {
    let encoded_user = urlencoding::encode(user);
    url.push_str(&format!("{}@", encoded_user));
  }

  url.push_str(host);
  url.push(':');
  url.push_str(&port.to_string());

  url
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
  // Set up panic handler to log panics before process exits
  std::panic::set_hook(Box::new(|panic_info| {
    log::error!("PANIC in proxy worker: {:?}", panic_info);
    if let Some(location) = panic_info.location() {
      log::error!(
        "Location: {}:{}:{}",
        location.file(),
        location.line(),
        location.column()
      );
    }
    if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
      log::error!("Message: {}", s);
    }
  }));

  let matches = Command::new("donut-proxy")
    .subcommand(
      Command::new("proxy")
        .about("Manage proxy servers")
        .subcommand(
          Command::new("start")
            .about("Start a proxy server")
            .arg(Arg::new("host").long("host").help("Upstream proxy host"))
            .arg(
              Arg::new("proxy-port")
                .long("proxy-port")
                .value_parser(clap::value_parser!(u16))
                .help("Upstream proxy port"),
            )
            .arg(
              Arg::new("type")
                .long("type")
                .help("Proxy type (http, https, socks4, socks5)"),
            )
            .arg(Arg::new("username").long("username").help("Proxy username"))
            .arg(Arg::new("password").long("password").help("Proxy password"))
            .arg(
              Arg::new("port")
                .short('p')
                .long("port")
                .value_parser(clap::value_parser!(u16))
                .help("Local port to use (random if not specified)"),
            )
            .arg(
              Arg::new("ignore-certificate")
                .long("ignore-certificate")
                .help("Ignore certificate errors for HTTPS proxies"),
            )
            .arg(
              Arg::new("upstream")
                .short('u')
                .long("upstream")
                .help("Upstream proxy URL (protocol://[username:password@]host:port)"),
            )
            .arg(
              Arg::new("profile-id")
                .long("profile-id")
                .help("ID of the profile this proxy is associated with"),
            ),
        )
        .subcommand(
          Command::new("stop")
            .about("Stop a proxy server")
            .arg(Arg::new("id").long("id").help("Proxy ID to stop"))
            .arg(
              Arg::new("upstream")
                .long("upstream")
                .help("Stop proxies with this upstream URL"),
            ),
        )
        .subcommand(Command::new("list").about("List all proxy servers")),
    )
    .subcommand(
      Command::new("proxy-worker")
        .about("Run a proxy worker process (internal use)")
        .arg(
          Arg::new("id")
            .long("id")
            .required(true)
            .help("Proxy configuration ID"),
        )
        .arg(Arg::new("action").required(true).help("Action (start)")),
    )
    .get_matches();

  if let Some(proxy_matches) = matches.subcommand_matches("proxy") {
    if let Some(start_matches) = proxy_matches.subcommand_matches("start") {
      let mut upstream_url: Option<String> = None;

      // Build upstream URL from individual components if provided
      if let (Some(host), Some(port), Some(proxy_type)) = (
        start_matches.get_one::<String>("host"),
        start_matches.get_one::<u16>("proxy-port"),
        start_matches.get_one::<String>("type"),
      ) {
        let username = start_matches.get_one::<String>("username");
        let password = start_matches.get_one::<String>("password");
        upstream_url = Some(build_proxy_url(
          proxy_type,
          host,
          *port,
          username.map(|s| s.as_str()),
          password.map(|s| s.as_str()),
        ));
      } else if let Some(upstream) = start_matches.get_one::<String>("upstream") {
        upstream_url = Some(upstream.clone());
      }

      let port = start_matches.get_one::<u16>("port").copied();
      let profile_id = start_matches.get_one::<String>("profile-id").cloned();

      match start_proxy_process_with_profile(upstream_url, port, profile_id).await {
        Ok(config) => {
          // Output the configuration as JSON for the Rust side to parse
          // Use println! here because this needs to go to stdout for parsing
          println!(
            "{}",
            serde_json::json!({
              "id": config.id,
              "localPort": config.local_port,
              "localUrl": config.local_url,
              "upstreamUrl": config.upstream_url,
            })
          );
          process::exit(0);
        }
        Err(e) => {
          eprintln!("Failed to start proxy: {}", e);
          process::exit(1);
        }
      }
    } else if let Some(stop_matches) = proxy_matches.subcommand_matches("stop") {
      if let Some(id) = stop_matches.get_one::<String>("id") {
        match stop_proxy_process(id).await {
          Ok(success) => {
            // Use println! here because this needs to go to stdout for parsing
            println!("{}", serde_json::json!({ "success": success }));
            process::exit(0);
          }
          Err(e) => {
            eprintln!("Failed to stop proxy: {}", e);
            process::exit(1);
          }
        }
      } else if let Some(upstream) = stop_matches.get_one::<String>("upstream") {
        // Find proxies with this upstream URL
        let configs = donutbrowser_lib::proxy_storage::list_proxy_configs();
        let matching_configs: Vec<_> = configs
          .iter()
          .filter(|config| config.upstream_url == *upstream)
          .collect();

        if matching_configs.is_empty() {
          eprintln!("No proxies found for {}", upstream);
          process::exit(1);
        }

        for config in matching_configs {
          let _ = stop_proxy_process(&config.id).await;
        }

        // Use println! here because this needs to go to stdout for parsing
        println!("{}", serde_json::json!({ "success": true }));
        process::exit(0);
      } else {
        // Stop all proxies
        match stop_all_proxy_processes().await {
          Ok(_) => {
            // Use println! here because this needs to go to stdout for parsing
            println!("{}", serde_json::json!({ "success": true }));
            process::exit(0);
          }
          Err(e) => {
            eprintln!("Failed to stop all proxies: {}", e);
            process::exit(1);
          }
        }
      }
    } else if proxy_matches.subcommand_matches("list").is_some() {
      let configs = donutbrowser_lib::proxy_storage::list_proxy_configs();
      // Use println! here because this needs to go to stdout for parsing
      println!("{}", serde_json::to_string(&configs).unwrap());
      process::exit(0);
    } else {
      log::error!("Invalid action. Use 'start', 'stop', or 'list'");
      process::exit(1);
    }
  } else if let Some(worker_matches) = matches.subcommand_matches("proxy-worker") {
    let id = worker_matches
      .get_one::<String>("id")
      .expect("id is required");
    let action = worker_matches
      .get_one::<String>("action")
      .expect("action is required");

    if action == "start" {
      // Set high priority so this process is killed last under resource pressure
      set_high_priority();

      log::error!("Proxy worker starting, looking for config id: {}", id);
      log::error!("Process PID: {}", std::process::id());

      let config = match get_proxy_config(id) {
        Some(config) => {
          log::error!(
            "Found config: id={}, port={:?}, upstream={}",
            config.id,
            config.local_port,
            config.upstream_url
          );
          config
        }
        None => {
          log::error!("Proxy configuration {} not found", id);
          process::exit(1);
        }
      };

      // Run the proxy server - this should never return (infinite loop)
      log::error!("Starting proxy server for config id: {}", id);
      if let Err(e) = run_proxy_server(config).await {
        log::error!("Failed to run proxy server: {}", e);
        log::error!("Error details: {:?}", e);
        process::exit(1);
      }
      // This should never be reached - run_proxy_server has an infinite loop
      log::error!("ERROR: Proxy server returned unexpectedly (this should never happen)");
      process::exit(1);
    } else {
      log::error!("Invalid action for proxy-worker. Use 'start'");
      process::exit(1);
    }
  } else {
    log::error!("No command specified");
    process::exit(1);
  }
}
