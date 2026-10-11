//! A change to many profiles at once: what bulk edit sends, and what a
//! template adds to each profile made from it.
//!
//! A field left out stays as it is. For the fields that can be empty, `null`
//! clears them. Every value is checked once, before any profile changes, so a
//! bad value changes nothing.

use super::types::BrowserProfile;
use serde::{Deserialize, Deserializer, Serialize};

/// Tells "sent as null" (clear) apart from "left out" (keep).
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
  D: Deserializer<'de>,
  T: Deserialize<'de>,
{
  Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfilePatch {
  #[serde(default, deserialize_with = "present")]
  pub status_id: Option<Option<String>>,
  #[serde(default)]
  pub tags_add: Vec<String>,
  #[serde(default)]
  pub tags_remove: Vec<String>,
  #[serde(default, deserialize_with = "present")]
  pub note: Option<Option<String>>,
  #[serde(default, deserialize_with = "present")]
  pub group_id: Option<Option<String>>,
  #[serde(default, deserialize_with = "present")]
  pub extension_group_id: Option<Option<String>>,
  #[serde(default, deserialize_with = "present")]
  pub dns_blocklist: Option<Option<String>>,
  #[serde(default, deserialize_with = "present")]
  pub dns_server: Option<Option<String>>,
  #[serde(default)]
  pub proxy_bypass_rules: Option<Vec<String>>,
  #[serde(default)]
  pub start_urls: Option<Vec<String>>,
  #[serde(default, deserialize_with = "present")]
  pub launch_hook: Option<Option<String>>,
  /// Left off ephemeral and password-protected profiles, where it means
  /// nothing: they never keep browsing data in the clear.
  #[serde(default)]
  pub clear_on_close: Option<bool>,
  #[serde(default)]
  pub randomize_fingerprint_on_launch: Option<bool>,
  #[serde(default, deserialize_with = "present")]
  pub webrtc_mode: Option<Option<String>>,
  #[serde(default)]
  pub block_images: Option<bool>,
  #[serde(default)]
  pub block_webgl: Option<bool>,
  #[serde(default)]
  pub restore_session: Option<bool>,
  /// Drop the stored device, so the next launch generates a new one.
  #[serde(default)]
  pub new_device: bool,
}

fn code(code: &str) -> String {
  crate::backend_error(code)
}

fn trimmed(value: Option<String>) -> Option<String> {
  value
    .map(|v| v.trim().to_string())
    .filter(|v| !v.is_empty())
}

impl ProfilePatch {
  /// The patch with every value checked and in its stored form.
  pub fn validated(mut self) -> Result<Self, String> {
    if let Some(status_id) = self.status_id.take() {
      let status_id = trimmed(status_id);
      if let Some(id) = &status_id {
        if !crate::profile_statuses::exists(id) {
          return Err(code("PROFILE_STATUS_NOT_FOUND"));
        }
      }
      self.status_id = Some(status_id);
    }
    if let Some(group_id) = self.group_id.take() {
      let group_id = trimmed(group_id);
      if let Some(id) = &group_id {
        let exists = crate::group_manager::GROUP_MANAGER
          .lock()
          .map_err(|_| code("INTERNAL_ERROR"))?
          .get_all_groups()
          .map_err(|e| crate::wrap_backend_error(e, "Failed to read groups"))?
          .iter()
          .any(|group| &group.id == id);
        if !exists {
          return Err(code("GROUP_NOT_FOUND"));
        }
      }
      self.group_id = Some(group_id);
    }
    if let Some(extension_group_id) = self.extension_group_id.take() {
      let extension_group_id = trimmed(extension_group_id);
      if let Some(id) = &extension_group_id {
        crate::extension_manager::EXTENSION_MANAGER
          .lock()
          .map_err(|_| code("INTERNAL_ERROR"))?
          .get_group(id)
          .map_err(|_| code("EXTENSION_GROUP_NOT_FOUND"))?;
      }
      self.extension_group_id = Some(extension_group_id);
    }
    if let Some(level) = self.dns_blocklist.take() {
      let level = trimmed(level).filter(|level| level != "none");
      if let Some(level) = &level {
        if crate::dns_blocklist::BlocklistLevel::parse_level(level).is_none() {
          return Err(code("DNS_BLOCKLIST_INVALID"));
        }
      }
      self.dns_blocklist = Some(level);
    }
    if let Some(dns_server) = self.dns_server.take() {
      self.dns_server = Some(crate::profile_dns::normalize_dns_server(dns_server)?);
    }
    if let Some(start_urls) = self.start_urls.take() {
      self.start_urls = Some(super::manager::checked_start_urls(start_urls)?);
    }
    if let Some(hook) = self.launch_hook.take() {
      self.launch_hook = Some(super::manager::validate_launch_hook(hook.as_deref())?);
    }
    if let Some(mode) = self.webrtc_mode.take() {
      let mode = match trimmed(mode) {
        None => None,
        Some(mode) => Some(
          crate::wayfern_manager::WebRtcMode::parse(&mode)
            .ok_or_else(|| code("WEBRTC_MODE_INVALID"))?
            .switch_value()
            .to_string(),
        ),
      };
      self.webrtc_mode = Some(mode);
    }
    if let Some(rules) = self.proxy_bypass_rules.take() {
      self.proxy_bypass_rules = Some(
        rules
          .into_iter()
          .map(|rule| rule.trim().to_string())
          .filter(|rule| !rule.is_empty())
          .collect(),
      );
    }
    self.tags_add = tidy_tags(std::mem::take(&mut self.tags_add));
    self.tags_remove = tidy_tags(std::mem::take(&mut self.tags_remove));
    Ok(self)
  }

  /// Whether the patch changes the group or extension group, which carry
  /// sync side effects beyond the profile itself.
  pub fn group_ids(&self) -> (Option<&str>, Option<&str>) {
    (
      self.group_id.as_ref().and_then(|g| g.as_deref()),
      self.extension_group_id.as_ref().and_then(|g| g.as_deref()),
    )
  }

  /// Change one profile. Returns whether anything changed. Only called on a
  /// patch that `validated` returned.
  pub fn apply_to(&self, profile: &mut BrowserProfile) -> bool {
    let before = serde_json::to_value(&*profile).ok();

    if let Some(status_id) = &self.status_id {
      profile.status_id = status_id.clone();
    }
    if !self.tags_add.is_empty() || !self.tags_remove.is_empty() {
      let mut tags: Vec<String> = profile
        .tags
        .iter()
        .filter(|tag| !self.tags_remove.contains(tag))
        .cloned()
        .collect();
      for tag in &self.tags_add {
        if !tags.contains(tag) {
          tags.push(tag.clone());
        }
      }
      profile.tags = tags;
    }
    if let Some(note) = &self.note {
      profile.note = trimmed(note.clone());
    }
    if let Some(group_id) = &self.group_id {
      profile.group_id = group_id.clone();
    }
    if let Some(extension_group_id) = &self.extension_group_id {
      profile.extension_group_id = extension_group_id.clone();
    }
    if let Some(level) = &self.dns_blocklist {
      profile.dns_blocklist = level.clone();
    }
    if let Some(dns_server) = &self.dns_server {
      profile.dns_server = dns_server.clone();
    }
    if let Some(rules) = &self.proxy_bypass_rules {
      profile.proxy_bypass_rules = rules.clone();
    }
    if let Some(start_urls) = &self.start_urls {
      profile.start_urls = start_urls.clone();
    }
    if let Some(hook) = &self.launch_hook {
      profile.launch_hook = hook.clone();
    }
    if let Some(clear) = self.clear_on_close {
      profile.clear_on_close = clear && !profile.ephemeral && !profile.password_protected;
    }

    let touches_device = self.randomize_fingerprint_on_launch.is_some()
      || self.webrtc_mode.is_some()
      || self.block_images.is_some()
      || self.block_webgl.is_some()
      || self.restore_session.is_some()
      || self.new_device;
    if touches_device {
      let config = profile.wayfern_config.get_or_insert_with(Default::default);
      if let Some(randomize) = self.randomize_fingerprint_on_launch {
        config.randomize_fingerprint_on_launch = Some(randomize);
      }
      if let Some(mode) = &self.webrtc_mode {
        config.webrtc_mode = mode.clone();
        // The legacy switch would otherwise still read as `block`.
        config.block_webrtc = None;
      }
      if let Some(block) = self.block_images {
        config.block_images = Some(block);
      }
      if let Some(block) = self.block_webgl {
        config.block_webgl = Some(block);
      }
      if let Some(restore) = self.restore_session {
        config.restore_session = Some(restore);
      }
      if self.new_device {
        config.fingerprint = None;
        config.identity_id = None;
        config.identity_baseline = None;
        config.identity_overrides = None;
      }
    }

    before != serde_json::to_value(&*profile).ok()
  }
}

fn tidy_tags(tags: Vec<String>) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  for tag in tags {
    let tag = tag.trim().to_string();
    if !tag.is_empty() && !out.contains(&tag) {
      out.push(tag);
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn patch(json: serde_json::Value) -> ProfilePatch {
    serde_json::from_value(json).unwrap()
  }

  #[test]
  fn null_clears_and_absence_keeps() {
    assert_eq!(patch(serde_json::json!({})).status_id, None);
    assert_eq!(
      patch(serde_json::json!({ "status_id": null })).status_id,
      Some(None)
    );
    assert_eq!(
      patch(serde_json::json!({ "status_id": "s1" })).status_id,
      Some(Some("s1".to_string()))
    );
    assert!(serde_json::from_value::<ProfilePatch>(serde_json::json!({ "name": "x" })).is_err());
  }

  #[test]
  fn a_patch_changes_only_what_it_names() {
    let mut profile = BrowserProfile {
      tags: vec!["keep".into(), "drop".into()],
      note: Some("old".into()),
      dns_blocklist: Some("light".into()),
      ..Default::default()
    };
    let change = ProfilePatch {
      tags_add: vec!["new".into(), "keep".into()],
      tags_remove: vec!["drop".into()],
      note: Some(None),
      webrtc_mode: Some(Some("block".into())),
      ..Default::default()
    };
    assert!(change.apply_to(&mut profile));
    assert_eq!(profile.tags, ["keep", "new"]);
    assert_eq!(profile.note, None);
    assert_eq!(
      profile.dns_blocklist.as_deref(),
      Some("light"),
      "not named, so kept"
    );
    assert_eq!(
      profile
        .wayfern_config
        .as_ref()
        .unwrap()
        .webrtc_mode
        .as_deref(),
      Some("block")
    );
    assert!(
      !change.apply_to(&mut profile),
      "applying twice changes nothing"
    );
  }

  #[test]
  fn a_new_device_drops_the_stored_identity_and_nothing_else() {
    let mut profile = BrowserProfile {
      wayfern_config: Some(crate::wayfern_manager::WayfernConfig {
        identity_id: Some("id".into()),
        identity_overrides: Some("{}".into()),
        location: Some("{\"timezone\":\"Europe/Berlin\"}".into()),
        os: Some("windows".into()),
        ..Default::default()
      }),
      ..Default::default()
    };
    assert!(ProfilePatch {
      new_device: true,
      ..Default::default()
    }
    .apply_to(&mut profile));
    let config = profile.wayfern_config.unwrap();
    assert_eq!(config.identity_id, None);
    assert_eq!(config.identity_overrides, None);
    assert_eq!(
      config.os.as_deref(),
      Some("windows"),
      "the claimed OS stays"
    );
    assert!(
      config.location.is_some(),
      "the location belongs to the route"
    );
  }

  #[test]
  fn clear_on_close_skips_profiles_where_it_means_nothing() {
    let change = ProfilePatch {
      clear_on_close: Some(true),
      ..Default::default()
    };
    let mut ephemeral = BrowserProfile {
      ephemeral: true,
      ..Default::default()
    };
    change.apply_to(&mut ephemeral);
    assert!(!ephemeral.clear_on_close);
    let mut plain = BrowserProfile::default();
    change.apply_to(&mut plain);
    assert!(plain.clear_on_close);
  }
}
