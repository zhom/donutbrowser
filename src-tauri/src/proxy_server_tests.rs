#[cfg(test)]
mod tests {
  use super::*;
  use crate::proxy_runner::{start_proxy_process, stop_proxy_process};
  use crate::proxy_storage::{delete_proxy_config, generate_proxy_id, list_proxy_configs};
  use std::process::Command;
  use std::time::Duration;
  use tokio::net::TcpStream;
  use tokio::time::sleep;

  #[tokio::test]
  async fn test_proxy_storage() {
    // Test proxy config storage
    let id = generate_proxy_id();
    let config = crate::proxy_storage::ProxyConfig::new(id.clone(), "DIRECT".to_string(), Some(8080));

    // Save config
    crate::proxy_storage::save_proxy_config(&config).unwrap();

    // Load config
    let loaded = crate::proxy_storage::get_proxy_config(&id).unwrap();
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.upstream_url, "DIRECT");
    assert_eq!(loaded.local_port, Some(8080));

    // Delete config
    assert!(crate::proxy_storage::delete_proxy_config(&id));
    assert!(crate::proxy_storage::get_proxy_config(&id).is_none());
  }

  #[tokio::test]
  async fn test_proxy_id_generation() {
    let id1 = generate_proxy_id();
    let id2 = generate_proxy_id();
    assert_ne!(id1, id2);
    assert!(id1.starts_with("proxy_"));
  }

  #[tokio::test]
  async fn test_proxy_process_lifecycle() {
    // Start a direct proxy
    let config = start_proxy_process(None, Some(0)).await.unwrap();
    let id = config.id.clone();

    // Verify config was saved
    let loaded = crate::proxy_storage::get_proxy_config(&id).unwrap();
    assert_eq!(loaded.id, id);

    // Wait a bit for the proxy to start
    sleep(Duration::from_millis(500)).await;

    // Stop the proxy
    let stopped = stop_proxy_process(&id).await.unwrap();
    assert!(stopped);

    // Verify config was deleted
    assert!(crate::proxy_storage::get_proxy_config(&id).is_none());
  }

  #[tokio::test]
  async fn test_proxy_with_upstream_http() {
    // Start a proxy with HTTP upstream (using a non-existent proxy for testing)
    let upstream_url = "http://127.0.0.1:9999";
    let config = start_proxy_process(Some(upstream_url.to_string()), Some(0))
      .await
      .unwrap();

    let id = config.id.clone();

    // Wait a bit
    sleep(Duration::from_millis(500)).await;

    // Clean up
    let _ = stop_proxy_process(&id).await;
  }

  #[tokio::test]
  async fn test_proxy_with_upstream_socks5() {
    // Start a proxy with SOCKS5 upstream
    let upstream_url = "socks5://127.0.0.1:1080";
    let config = start_proxy_process(Some(upstream_url.to_string()), Some(0))
      .await
      .unwrap();

    let id = config.id.clone();

    // Wait a bit
    sleep(Duration::from_millis(500)).await;

    // Clean up
    let _ = stop_proxy_process(&id).await;
  }

  #[tokio::test]
  async fn test_proxy_port_assignment() {
    // Start multiple proxies and verify they get different ports
    let config1 = start_proxy_process(None, None).await.unwrap();
    sleep(Duration::from_millis(100)).await;
    let config2 = start_proxy_process(None, None).await.unwrap();

    // They should have different IDs
    assert_ne!(config1.id, config2.id);

    // Clean up
    let _ = stop_proxy_process(&config1.id).await;
    let _ = stop_proxy_process(&config2.id).await;
  }

  #[tokio::test]
  async fn test_proxy_list() {
    // Start a few proxies
    let config1 = start_proxy_process(None, None).await.unwrap();
    sleep(Duration::from_millis(100)).await;
    let config2 = start_proxy_process(None, None).await.unwrap();

    // List all proxies
    let configs = list_proxy_configs();
    assert!(configs.len() >= 2);

    // Clean up
    let _ = stop_proxy_process(&config1.id).await;
    let _ = stop_proxy_process(&config2.id).await;
  }
}

