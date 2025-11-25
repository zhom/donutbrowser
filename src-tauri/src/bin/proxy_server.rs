use clap::{Arg, Command};
use donutbrowser::proxy_runner::{
  start_proxy_process, stop_all_proxy_processes, stop_proxy_process,
};
use donutbrowser::proxy_server::run_proxy_server;
use donutbrowser::proxy_storage::get_proxy_config;
use std::process;

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
    eprintln!("PANIC in proxy worker: {:?}", panic_info);
    if let Some(location) = panic_info.location() {
      eprintln!(
        "Location: {}:{}:{}",
        location.file(),
        location.line(),
        location.column()
      );
    }
    if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
      eprintln!("Message: {}", s);
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

      match start_proxy_process(upstream_url, port).await {
        Ok(config) => {
          // Output the configuration as JSON for the Rust side to parse
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
        let configs = donutbrowser::proxy_storage::list_proxy_configs();
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

        println!("{}", serde_json::json!({ "success": true }));
        process::exit(0);
      } else {
        // Stop all proxies
        match stop_all_proxy_processes().await {
          Ok(_) => {
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
      let configs = donutbrowser::proxy_storage::list_proxy_configs();
      println!("{}", serde_json::to_string(&configs).unwrap());
      process::exit(0);
    } else {
      eprintln!("Invalid action. Use 'start', 'stop', or 'list'");
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
      eprintln!("Proxy worker starting, looking for config id: {}", id);
      eprintln!("Process PID: {}", std::process::id());

      let config = match get_proxy_config(id) {
        Some(config) => {
          eprintln!(
            "Found config: id={}, port={:?}, upstream={}",
            config.id, config.local_port, config.upstream_url
          );
          config
        }
        None => {
          eprintln!("Proxy configuration {} not found", id);
          process::exit(1);
        }
      };

      // Run the proxy server - this should never return (infinite loop)
      eprintln!("Starting proxy server for config id: {}", id);
      if let Err(e) = run_proxy_server(config).await {
        eprintln!("Failed to run proxy server: {}", e);
        eprintln!("Error details: {:?}", e);
        process::exit(1);
      }
      // This should never be reached - run_proxy_server has an infinite loop
      eprintln!("ERROR: Proxy server returned unexpectedly (this should never happen)");
      process::exit(1);
    } else {
      eprintln!("Invalid action for proxy-worker. Use 'start'");
      process::exit(1);
    }
  } else {
    eprintln!("No command specified");
    process::exit(1);
  }
}
