//! What MCP tools answer with, kept small: an agent pays for every byte in
//! its context.
//!
//! - `tool_json`: compact JSON text plus `structuredContent`, a list cut to fit
//!   when it is too large, with a note that says so.
//! - `list_profiles`: a short summary per profile, filters and paging, names
//!   next to ids so an agent needs no second call to read them.
//! - `profile_detail`: one profile without device internals, secrets or
//!   another person's email.

use crate::profile::BrowserProfile;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// The most text one tool answer carries. Past it a list is cut to fit, so
/// one call can not fill an agent's context.
pub const MAX_TOOL_TEXT_BYTES: usize = 100_000;

/// A tool's answer: compact JSON text that every client reads, and the same
/// value as `structuredContent` for clients that read that instead (the spec
/// allows an object there, not a list).
pub fn tool_json(value: Value) -> Value {
  let text = serde_json::to_string(&value).unwrap_or_default();
  if text.len() <= MAX_TOOL_TEXT_BYTES {
    let mut result = json!({ "content": [{ "type": "text", "text": text }] });
    if value.is_object() {
      result["structuredContent"] = value;
    }
    return result;
  }
  let Value::Array(items) = value else {
    // One record this large has a large field the caller asked for by id;
    // cutting it would make it wrong, so it goes whole, once.
    return json!({ "content": [{ "type": "text", "text": text }] });
  };
  let total = items.len();
  let mut kept = Vec::new();
  let mut size = 2;
  for item in items {
    let item_size = serde_json::to_string(&item).map_or(0, |t| t.len()) + 1;
    if size + item_size > MAX_TOOL_TEXT_BYTES {
      break;
    }
    size += item_size;
    kept.push(item);
  }
  let shown = kept.len();
  json!({ "content": [
    { "type": "text", "text": serde_json::to_string(&kept).unwrap_or_default() },
    {
      "type": "text",
      "text": format!(
        "Truncated: {shown} of {total} items shown. Narrow the request (a filter, a limit or one id) to see the rest."
      )
    }
  ]})
}

/// Names for the ids a profile points at, read once per call.
#[derive(Default)]
pub struct Names {
  pub groups: HashMap<String, String>,
  pub proxies: HashMap<String, String>,
  pub vpns: HashMap<String, String>,
  pub statuses: HashMap<String, String>,
  /// Profiles running on the remote fleet.
  pub remote: HashSet<String>,
}

pub const DEFAULT_LIST_LIMIT: usize = 50;
pub const MAX_LIST_LIMIT: usize = 200;
const NOTE_CHARS: usize = 200;

#[derive(Debug, Default, PartialEq)]
pub struct ProfileQuery {
  pub text: Option<String>,
  pub group: Option<String>,
  pub tag: Option<String>,
  pub running: Option<bool>,
  pub limit: usize,
  pub offset: usize,
}

impl ProfileQuery {
  /// From the tool's arguments. A bad cursor or limit is an error, not a
  /// silent first page.
  pub fn from_arguments(arguments: &Value) -> Result<Self, String> {
    let text_of = |key: &str| {
      arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    };
    let limit = match arguments.get("limit") {
      None | Some(Value::Null) => DEFAULT_LIST_LIMIT,
      Some(value) => value
        .as_u64()
        .filter(|limit| (1..=MAX_LIST_LIMIT as u64).contains(limit))
        .ok_or_else(|| format!("limit must be a whole number from 1 to {MAX_LIST_LIMIT}"))?
        as usize,
    };
    let offset = match text_of("cursor") {
      None => 0,
      Some(cursor) => cursor
        .parse::<usize>()
        .map_err(|_| "cursor must be a next_cursor from an earlier page".to_string())?,
    };
    Ok(Self {
      text: text_of("query").map(|q| q.to_lowercase()),
      group: text_of("group"),
      tag: text_of("tag"),
      running: arguments.get("running").and_then(Value::as_bool),
      limit,
      offset,
    })
  }
}

fn running_state(profile: &BrowserProfile, names: &Names) -> &'static str {
  if profile.process_id.is_some() {
    "local"
  } else if names.remote.contains(&profile.id.to_string()) {
    "remote"
  } else {
    "stopped"
  }
}

fn short_note(note: &str) -> String {
  if note.chars().count() <= NOTE_CHARS {
    return note.to_string();
  }
  let cut: String = note.chars().take(NOTE_CHARS).collect();
  format!("{cut}…")
}

fn summary(profile: &BrowserProfile, names: &Names) -> Value {
  let id = profile.id.to_string();
  let named = |map: &HashMap<String, String>, id: &Option<String>| {
    id.as_ref()
      .map(|id| json!({ "id": id, "name": map.get(id) }))
  };
  let route = match (&profile.proxy_id, &profile.vpn_id) {
    (Some(proxy), _) => json!({ "kind": "proxy", "id": proxy, "name": names.proxies.get(proxy) }),
    (None, Some(vpn)) => json!({ "kind": "vpn", "id": vpn, "name": names.vpns.get(vpn) }),
    (None, None) => json!({ "kind": "direct" }),
  };
  json!({
    "id": id,
    "name": profile.name,
    "group": named(&names.groups, &profile.group_id),
    "tags": profile.tags,
    "status": profile.status_id.as_ref().and_then(|s| names.statuses.get(s)),
    "route": route,
    "running": running_state(profile, names),
    "last_launch": profile.last_launch,
    "note": profile.note.as_deref().map(short_note),
  })
}

fn matches(profile: &BrowserProfile, names: &Names, query: &ProfileQuery) -> bool {
  if let Some(text) = &query.text {
    let hit = profile.name.to_lowercase().contains(text)
      || profile
        .note
        .as_ref()
        .is_some_and(|note| note.to_lowercase().contains(text))
      || profile
        .tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(text));
    if !hit {
      return false;
    }
  }
  if let Some(group) = &query.group {
    let ok = match &profile.group_id {
      None => group.eq_ignore_ascii_case("none"),
      Some(id) => {
        id == group
          || names
            .groups
            .get(id)
            .is_some_and(|name| name.eq_ignore_ascii_case(group))
      }
    };
    if !ok {
      return false;
    }
  }
  if let Some(tag) = &query.tag {
    if !profile.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
      return false;
    }
  }
  if let Some(running) = query.running {
    if (running_state(profile, names) != "stopped") != running {
      return false;
    }
  }
  true
}

/// The `list_profiles` answer. Only Wayfern profiles can be driven over MCP;
/// the others are counted, so an agent knows they exist.
pub fn list_profiles(profiles: &[BrowserProfile], names: &Names, query: &ProfileQuery) -> Value {
  let hidden = profiles.iter().filter(|p| p.browser != "wayfern").count();
  let mut found: Vec<&BrowserProfile> = profiles
    .iter()
    .filter(|p| p.browser == "wayfern" && matches(p, names, query))
    .collect();
  found.sort_by(|a, b| {
    a.name
      .to_lowercase()
      .cmp(&b.name.to_lowercase())
      .then_with(|| a.id.cmp(&b.id))
  });
  let total = found.len();
  let page: Vec<Value> = found
    .iter()
    .skip(query.offset)
    .take(query.limit)
    .map(|profile| summary(profile, names))
    .collect();
  let next = query.offset + page.len();
  let mut answer = json!({
    "profiles": page,
    "total": total,
    "next_cursor": (next < total).then(|| next.to_string()),
  });
  if hidden > 0 {
    answer["hidden"] = json!({
      "count": hidden,
      "reason": "Not Wayfern profiles: they can not be driven over MCP.",
    });
  }
  answer
}

/// One profile for `get_profile`. Left out: the device itself (ask
/// `get_profile_fingerprint`), the encryption salt, the creator's email and
/// id, and the launch hook URL, which can carry a token.
pub fn profile_detail(profile: &BrowserProfile, names: &Names) -> Value {
  let mut value = serde_json::to_value(profile).unwrap_or_default();
  if let Some(object) = value.as_object_mut() {
    for field in [
      "encryption_salt",
      "created_by_email",
      "created_by_id",
      "launch_hook",
      "process_id",
    ] {
      object.remove(field);
    }
    object.insert(
      "has_launch_hook".into(),
      json!(profile.launch_hook.is_some()),
    );
    object.insert("running".into(), json!(running_state(profile, names)));
    if let Some(Value::Object(config)) = object.get_mut("wayfern_config") {
      for field in [
        "fingerprint",
        "identity_baseline",
        "identity_overrides",
        "persona",
        "location",
        "geo_proxy_signature",
        "proxy",
      ] {
        config.remove(field);
      }
    }
    let summary = summary(profile, names);
    for field in ["group", "status", "route"] {
      object.insert(field.into(), summary[field].clone());
    }
  }
  value
}

/// The `outputSchema` of a tool whose answer has a fixed shape.
pub fn output_schema_for(tool: &str) -> Option<Value> {
  match tool {
    "list_profiles" => Some(json!({
      "type": "object",
      "properties": {
        "profiles": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "id": { "type": "string" },
              "name": { "type": "string" },
              "group": {
                "type": ["object", "null"],
                "properties": { "id": { "type": "string" }, "name": { "type": ["string", "null"] } }
              },
              "tags": { "type": "array", "items": { "type": "string" } },
              "status": { "type": ["string", "null"] },
              "route": {
                "type": "object",
                "properties": {
                  "kind": { "type": "string", "enum": ["proxy", "vpn", "direct"] },
                  "id": { "type": "string" },
                  "name": { "type": ["string", "null"] }
                },
                "required": ["kind"]
              },
              "running": { "type": "string", "enum": ["local", "remote", "stopped"] },
              "last_launch": { "type": ["integer", "null"] },
              "note": { "type": ["string", "null"] }
            },
            "required": ["id", "name", "tags", "route", "running"]
          }
        },
        "total": { "type": "integer" },
        "next_cursor": { "type": ["string", "null"] },
        "hidden": {
          "type": "object",
          "properties": { "count": { "type": "integer" }, "reason": { "type": "string" } }
        }
      },
      "required": ["profiles", "total", "next_cursor"]
    })),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn profile(name: &str) -> BrowserProfile {
    BrowserProfile {
      id: uuid::Uuid::new_v4(),
      name: name.to_string(),
      browser: "wayfern".to_string(),
      ..Default::default()
    }
  }

  fn query(arguments: Value) -> ProfileQuery {
    ProfileQuery::from_arguments(&arguments).unwrap()
  }

  #[test]
  fn a_summary_is_small_and_names_what_ids_point_at() {
    let mut shop = profile("Shop");
    shop.group_id = Some("g1".into());
    shop.proxy_id = Some("p1".into());
    shop.status_id = Some("s1".into());
    shop.tags = vec!["eu".into()];
    shop.note = Some("x".repeat(500));
    shop.launch_hook = Some("https://hook.example/?token=secret".into());
    shop.wayfern_config = Some(crate::wayfern_manager::WayfernConfig {
      identity_overrides: Some("{\"a\":\"".to_string() + &"z".repeat(4000) + "\"}"),
      ..Default::default()
    });
    let names = Names {
      groups: HashMap::from([("g1".into(), "Europe".into())]),
      proxies: HashMap::from([("p1".into(), "Berlin".into())]),
      statuses: HashMap::from([("s1".into(), "Warming".into())]),
      ..Default::default()
    };
    let answer = list_profiles(&[shop.clone()], &names, &query(json!({})));
    let row = &answer["profiles"][0];
    assert_eq!(row["group"]["name"], "Europe");
    assert_eq!(row["route"]["name"], "Berlin");
    assert_eq!(row["status"], "Warming");
    assert_eq!(row["running"], "stopped");
    assert_eq!(row["note"].as_str().unwrap().chars().count(), 201);
    let text = serde_json::to_string(row).unwrap();
    assert!(text.len() < 700, "summary is {} bytes", text.len());
    assert!(!text.contains("secret") && !text.contains("zzzz"));

    let detail = serde_json::to_string(&profile_detail(&shop, &names)).unwrap();
    assert!(!detail.contains("secret"), "the hook URL can carry a token");
    assert!(
      !detail.contains("zzzz"),
      "device internals are a separate tool"
    );
    assert!(detail.contains("\"has_launch_hook\":true"));
  }

  #[test]
  fn filters_and_pages_combine() {
    let mut profiles: Vec<BrowserProfile> =
      (1..=5).map(|i| profile(&format!("Mail {i}"))).collect();
    profiles[0].process_id = Some(42);
    profiles[1].tags = vec!["VIP".into()];
    profiles[2].group_id = Some("g1".into());
    let mut camoufox = profile("Old");
    camoufox.browser = "camoufox".into();
    profiles.push(camoufox);
    let names = Names {
      groups: HashMap::from([("g1".into(), "Europe".into())]),
      remote: HashSet::from([profiles[3].id.to_string()]),
      ..Default::default()
    };

    let first = list_profiles(&profiles, &names, &query(json!({ "limit": 2 })));
    assert_eq!(first["total"], 5);
    assert_eq!(first["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(first["next_cursor"], "2");
    assert_eq!(first["hidden"]["count"], 1);
    let last = list_profiles(
      &profiles,
      &names,
      &query(json!({ "limit": 2, "cursor": "4" })),
    );
    assert_eq!(last["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(last["next_cursor"], Value::Null);

    let running = list_profiles(&profiles, &names, &query(json!({ "running": true })));
    let states: Vec<&str> = running["profiles"]
      .as_array()
      .unwrap()
      .iter()
      .map(|p| p["running"].as_str().unwrap())
      .collect();
    assert_eq!(states, ["local", "remote"]);
    for (arguments, expected) in [
      (json!({ "tag": "vip" }), 1),
      (json!({ "group": "europe" }), 1),
      (json!({ "group": "g1" }), 1),
      (json!({ "group": "none" }), 4),
      (json!({ "query": "MAIL 5" }), 1),
    ] {
      assert_eq!(
        list_profiles(&profiles, &names, &query(arguments.clone()))["total"],
        expected,
        "{arguments}"
      );
    }
    assert!(ProfileQuery::from_arguments(&json!({ "limit": 0 })).is_err());
    assert!(ProfileQuery::from_arguments(&json!({ "cursor": "abc" })).is_err());
  }

  #[test]
  fn objects_get_structured_content_and_long_lists_are_cut_with_a_note() {
    let small = tool_json(json!({ "a": 1 }));
    assert_eq!(small["structuredContent"]["a"], 1);
    assert_eq!(small["content"][0]["text"], "{\"a\":1}");
    assert!(tool_json(json!([1, 2]))["structuredContent"].is_null());

    let big: Vec<Value> = (0..5000)
      .map(|i| json!({ "i": i, "pad": "x".repeat(40) }))
      .collect();
    let answer = tool_json(Value::Array(big));
    let text = answer["content"][0]["text"].as_str().unwrap();
    assert!(text.len() <= MAX_TOOL_TEXT_BYTES);
    let kept: Vec<Value> = serde_json::from_str(text).unwrap();
    assert!(kept.len() < 5000);
    assert!(answer["content"][1]["text"]
      .as_str()
      .unwrap()
      .starts_with(&format!("Truncated: {} of 5000", kept.len())));
  }

  #[test]
  fn the_list_schema_describes_what_the_list_returns() {
    let schema = output_schema_for("list_profiles").unwrap();
    let names = Names::default();
    let answer = list_profiles(&[profile("A")], &names, &query(json!({})));
    for key in schema["required"].as_array().unwrap() {
      assert!(answer.get(key.as_str().unwrap()).is_some(), "{key}");
    }
    let row = &answer["profiles"][0];
    for key in schema["properties"]["profiles"]["items"]["required"]
      .as_array()
      .unwrap()
    {
      assert!(row.get(key.as_str().unwrap()).is_some(), "{key}");
    }
  }
}
