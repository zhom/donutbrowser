//! What Donut has running outside its own process, as `key=[id@pid ...]`
//! fields for one log line. Logged at startup (what an earlier run left) and at
//! exit (what outlives this run), so a report that Donut "keeps running" can be
//! answered from the log alone.

use crate::proxy_storage::is_process_running;

pub fn summary() -> String {
  let browsers = crate::profile::ProfileManager::instance()
    .list_profiles()
    .unwrap_or_default()
    .into_iter()
    .filter_map(|profile| {
      let pid = profile.process_id.filter(|pid| is_process_running(*pid))?;
      Some((profile.id.to_string(), pid))
    });
  let proxies = crate::proxy_storage::list_proxy_configs()
    .into_iter()
    .filter_map(|config| Some((config.id, config.pid?)));
  let vpns = crate::vpn_worker_storage::list_vpn_worker_configs()
    .into_iter()
    .filter_map(|config| Some((config.id, config.pid?)));
  let xrays = crate::xray_worker_storage::list_xray_worker_configs()
    .into_iter()
    .filter_map(|config| Some((config.id, config.pid?)));

  let mut line = format!(
    "browsers={} proxy_workers={} vpn_workers={} xray_workers={}",
    alive(browsers),
    alive(proxies),
    alive(vpns),
    alive(xrays)
  );
  if let Some(mounted) = crate::ephemeral_dirs::ram_disk_mounted() {
    line.push_str(if mounted {
      " ram_disk=mounted"
    } else {
      " ram_disk=absent"
    });
  }
  line
}

fn alive(entries: impl Iterator<Item = (String, u32)>) -> String {
  let listed: Vec<String> = entries
    .filter(|(_, pid)| is_process_running(*pid))
    .map(|(id, pid)| format!("{id}@{pid}"))
    .collect();
  format!("[{}]", listed.join(" "))
}

#[cfg(test)]
mod tests {
  use super::alive;

  #[test]
  fn only_live_processes_are_listed() {
    let own = std::process::id();
    let listed = alive([("live".to_string(), own), ("gone".to_string(), u32::MAX)].into_iter());
    assert_eq!(listed, format!("[live@{own}]"));
    assert_eq!(alive(std::iter::empty()), "[]");
  }
}
