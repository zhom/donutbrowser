//! Handing a fleet of profiles one proxy each.
//!
//! Fifty profiles and fifty residential proxies is fifty dialogs by hand. This
//! pairs them positionally instead — profile 1 to proxy 1, profile 2 to proxy
//! 2 — and it never wraps around: when the two lists differ in length the
//! remainder is reported rather than reused, because silently giving two
//! profiles the same exit is the one outcome a fleet owner is buying separate
//! proxies to avoid.
//!
//! The pairing rule lives here as one pure function so the dialog's preview,
//! the counts it shows, and the assignment that is finally applied all come
//! from the same code. The apply step takes explicit pairs, so a caller that
//! wants a different arrangement — REST, MCP, or a user who ticked boxes by
//! hand — is not forced through the default.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

/// One profile and the proxy it should end up on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProxyPair {
  pub profile_id: String,
  pub proxy_id: String,
}

/// A profile as the pairing rule sees it.
#[derive(Debug, Clone)]
pub struct ProfileCandidate {
  pub id: String,
  /// Its browser is alive on this machine, so its proxy cannot be changed.
  pub running: bool,
}

/// What a distribution would do, before anything is written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionPlan {
  /// The assignments, in the order the profiles were given.
  pub pairs: Vec<ProxyPair>,
  /// Chosen profiles that no proxy was left for.
  pub unpaired_profile_ids: Vec<String>,
  /// Chosen proxies that no profile was left for.
  pub unused_proxy_ids: Vec<String>,
  /// Chosen profiles refused because their browser is running.
  pub running_profile_ids: Vec<String>,
  /// Chosen proxies withheld because a profile outside this distribution
  /// already uses them and sharing was not allowed.
  pub shared_proxy_ids: Vec<String>,
}

/// Pair profiles to proxies one to one.
///
/// `assigned_elsewhere` is the set of proxies held by profiles that are not
/// part of this distribution. With `allow_sharing` off those proxies are taken
/// out of the pool, so a run cannot quietly put a second profile behind an exit
/// that is already in use. A proxy listed twice by the caller is the same
/// hazard and is deduplicated the same way.
pub fn plan(
  profiles: &[ProfileCandidate],
  proxy_ids: &[String],
  allow_sharing: bool,
  assigned_elsewhere: &HashSet<String>,
) -> DistributionPlan {
  let mut plan = DistributionPlan::default();

  let mut eligible = Vec::with_capacity(profiles.len());
  for profile in profiles {
    if profile.running {
      plan.running_profile_ids.push(profile.id.clone());
    } else {
      eligible.push(profile.id.clone());
    }
  }

  let mut pool: Vec<String> = Vec::with_capacity(proxy_ids.len());
  let mut seen: HashSet<&str> = HashSet::new();
  for proxy_id in proxy_ids {
    if !seen.insert(proxy_id.as_str()) {
      // The same proxy twice in one list is sharing spelled differently.
      if !allow_sharing {
        plan.shared_proxy_ids.push(proxy_id.clone());
        continue;
      }
    }
    if !allow_sharing && assigned_elsewhere.contains(proxy_id) {
      plan.shared_proxy_ids.push(proxy_id.clone());
      continue;
    }
    pool.push(proxy_id.clone());
  }

  let paired = eligible.len().min(pool.len());
  for index in 0..paired {
    plan.pairs.push(ProxyPair {
      profile_id: eligible[index].clone(),
      proxy_id: pool[index].clone(),
    });
  }
  plan.unpaired_profile_ids = eligible[paired..].to_vec();
  plan.unused_proxy_ids = pool[paired..].to_vec();
  plan
}

/// What happened to one profile in an apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProxyAssignmentResult {
  pub profile_id: String,
  pub proxy_id: String,
  pub ok: bool,
  /// A `{"code": ...}` payload when `ok` is false, otherwise null.
  pub error: Option<String>,
}

fn failed(pair: &ProxyPair, code: &str) -> ProxyAssignmentResult {
  ProxyAssignmentResult {
    profile_id: pair.profile_id.clone(),
    proxy_id: pair.proxy_id.clone(),
    ok: false,
    error: Some(serde_json::json!({ "code": code }).to_string()),
  }
}

/// Apply explicit pairs, one profile at a time, and report each outcome.
///
/// A profile that cannot be moved never stops the rest: fifty assignments in
/// which the one running profile fails is a useful answer, and an all-or-
/// nothing abort halfway through a fleet is not.
pub async fn apply_pairs(
  app_handle: tauri::AppHandle,
  pairs: &[ProxyPair],
) -> Vec<ProxyAssignmentResult> {
  let manager = crate::profile::ProfileManager::instance();
  let known_proxies: HashSet<String> = crate::proxy_manager::PROXY_MANAGER
    .get_stored_proxies()
    .into_iter()
    .map(|proxy| proxy.id)
    .collect();

  let mut results = Vec::with_capacity(pairs.len());
  let mut already_paired: HashSet<&str> = HashSet::new();

  for pair in pairs {
    if !already_paired.insert(pair.profile_id.as_str()) {
      // Two proxies for one profile is not an assignment, it is a mistake in
      // the request, and quietly applying the last one hides it.
      results.push(failed(pair, "PROFILE_PAIRED_TWICE"));
      continue;
    }
    if !known_proxies.contains(&pair.proxy_id) {
      results.push(failed(pair, "PROXY_NOT_FOUND"));
      continue;
    }

    // Re-read on every pair: an earlier assignment in this same batch, or a
    // browser someone started while the dialog was open, has to be visible.
    let profile = match manager.list_profiles() {
      Ok(profiles) => profiles
        .into_iter()
        .find(|profile| profile.id.to_string() == pair.profile_id),
      Err(e) => {
        log::warn!("Could not list profiles while distributing proxies: {e}");
        None
      }
    };
    let Some(profile) = profile else {
      results.push(failed(pair, "PROFILE_NOT_FOUND"));
      continue;
    };
    if crate::profile::trash::is_running_locally(&profile) {
      results.push(failed(pair, "PROFILE_RUNNING"));
      continue;
    }

    match manager
      .update_profile_proxy(
        app_handle.clone(),
        &pair.profile_id,
        Some(pair.proxy_id.clone()),
      )
      .await
    {
      Ok(_) => results.push(ProxyAssignmentResult {
        profile_id: pair.profile_id.clone(),
        proxy_id: pair.proxy_id.clone(),
        ok: true,
        error: None,
      }),
      Err(e) => results.push(ProxyAssignmentResult {
        profile_id: pair.profile_id.clone(),
        proxy_id: pair.proxy_id.clone(),
        ok: false,
        error: Some(e.to_string()),
      }),
    }
  }

  results
}

/// Build the pairing rule's view of the world from what is on disk.
fn candidates(
  profile_ids: &[String],
) -> Result<(Vec<ProfileCandidate>, HashSet<String>), Box<dyn std::error::Error>> {
  let profiles = crate::profile::ProfileManager::instance().list_profiles()?;
  let chosen: HashSet<&str> = profile_ids.iter().map(String::as_str).collect();

  let assigned_elsewhere = profiles
    .iter()
    .filter(|profile| !chosen.contains(profile.id.to_string().as_str()))
    .filter_map(|profile| profile.proxy_id.clone())
    .collect();

  let ordered = profile_ids
    .iter()
    .map(|id| {
      let found = profiles.iter().find(|p| p.id.to_string() == *id);
      ProfileCandidate {
        id: id.clone(),
        // A profile that is not on disk cannot be launched either, so it is
        // simply never paired; the plan reports it as unpaired.
        running: found.is_none_or(crate::profile::trash::is_running_locally),
      }
    })
    .collect();

  Ok((ordered, assigned_elsewhere))
}

/// Tauri command: what would happen, without touching anything.
#[tauri::command]
pub async fn plan_proxy_distribution(
  profile_ids: Vec<String>,
  proxy_ids: Vec<String>,
  allow_sharing: bool,
) -> Result<DistributionPlan, String> {
  let (profiles, assigned_elsewhere) = candidates(&profile_ids).map_err(|e| e.to_string())?;
  Ok(plan(
    &profiles,
    &proxy_ids,
    allow_sharing,
    &assigned_elsewhere,
  ))
}

/// Tauri command: apply the pairs and report every profile's outcome.
#[tauri::command]
pub async fn distribute_proxies_to_profiles(
  app_handle: tauri::AppHandle,
  pairs: Vec<ProxyPair>,
) -> Result<Vec<ProxyAssignmentResult>, String> {
  Ok(apply_pairs(app_handle, &pairs).await)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn free(id: &str) -> ProfileCandidate {
    ProfileCandidate {
      id: id.to_string(),
      running: false,
    }
  }

  fn running(id: &str) -> ProfileCandidate {
    ProfileCandidate {
      id: id.to_string(),
      running: true,
    }
  }

  fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
  }

  fn assigned(values: &[&str]) -> HashSet<String> {
    values.iter().map(|v| v.to_string()).collect()
  }

  #[test]
  fn equal_lists_pair_one_to_one_in_order() {
    let plan = plan(
      &[free("p1"), free("p2"), free("p3")],
      &ids(&["x1", "x2", "x3"]),
      false,
      &HashSet::new(),
    );
    assert_eq!(
      plan.pairs,
      vec![
        ProxyPair {
          profile_id: "p1".into(),
          proxy_id: "x1".into()
        },
        ProxyPair {
          profile_id: "p2".into(),
          proxy_id: "x2".into()
        },
        ProxyPair {
          profile_id: "p3".into(),
          proxy_id: "x3".into()
        },
      ]
    );
    assert!(plan.unpaired_profile_ids.is_empty());
    assert!(plan.unused_proxy_ids.is_empty());
  }

  #[test]
  fn more_profiles_than_proxies_leaves_the_remainder_alone() {
    // The failure this guards: wrapping around, which would put p3 and p4 on
    // the same exits as p1 and p2 without anyone asking for it.
    let plan = plan(
      &[free("p1"), free("p2"), free("p3"), free("p4")],
      &ids(&["x1", "x2"]),
      false,
      &HashSet::new(),
    );
    assert_eq!(plan.pairs.len(), 2);
    assert_eq!(plan.unpaired_profile_ids, ids(&["p3", "p4"]));
    assert!(plan.unused_proxy_ids.is_empty());
  }

  #[test]
  fn more_proxies_than_profiles_reports_the_leftovers() {
    let plan = plan(
      &[free("p1")],
      &ids(&["x1", "x2", "x3"]),
      false,
      &HashSet::new(),
    );
    assert_eq!(plan.pairs.len(), 1);
    assert_eq!(plan.unused_proxy_ids, ids(&["x2", "x3"]));
    assert!(plan.unpaired_profile_ids.is_empty());
  }

  #[test]
  fn a_proxy_another_profile_holds_is_withheld_until_sharing_is_allowed() {
    let profiles = [free("p1"), free("p2")];
    let proxies = ids(&["x1", "x2"]);
    let elsewhere = assigned(&["x1"]);

    let strict = plan(&profiles, &proxies, false, &elsewhere);
    assert_eq!(strict.shared_proxy_ids, ids(&["x1"]));
    assert_eq!(
      strict.pairs,
      vec![ProxyPair {
        profile_id: "p1".into(),
        proxy_id: "x2".into()
      }]
    );
    assert_eq!(strict.unpaired_profile_ids, ids(&["p2"]));

    let permissive = plan(&profiles, &proxies, true, &elsewhere);
    assert!(permissive.shared_proxy_ids.is_empty());
    assert_eq!(permissive.pairs.len(), 2);
    assert_eq!(permissive.pairs[0].proxy_id, "x1");
  }

  #[test]
  fn the_same_proxy_listed_twice_is_sharing_too() {
    let strict = plan(
      &[free("p1"), free("p2")],
      &ids(&["x1", "x1"]),
      false,
      &HashSet::new(),
    );
    assert_eq!(strict.pairs.len(), 1);
    assert_eq!(strict.shared_proxy_ids, ids(&["x1"]));
    assert_eq!(strict.unpaired_profile_ids, ids(&["p2"]));

    let permissive = plan(
      &[free("p1"), free("p2")],
      &ids(&["x1", "x1"]),
      true,
      &HashSet::new(),
    );
    assert_eq!(permissive.pairs.len(), 2);
    assert_eq!(permissive.pairs[1].proxy_id, "x1");
  }

  #[test]
  fn a_running_profile_is_named_and_never_paired() {
    let plan = plan(
      &[free("p1"), running("p2"), free("p3")],
      &ids(&["x1", "x2"]),
      false,
      &HashSet::new(),
    );
    assert_eq!(plan.running_profile_ids, ids(&["p2"]));
    assert_eq!(
      plan.pairs,
      vec![
        ProxyPair {
          profile_id: "p1".into(),
          proxy_id: "x1".into()
        },
        // p3 takes the second proxy: the running profile is skipped, it does
        // not consume a proxy and leave a hole behind it.
        ProxyPair {
          profile_id: "p3".into(),
          proxy_id: "x2".into()
        },
      ]
    );
    assert!(plan.unpaired_profile_ids.is_empty());
  }

  #[test]
  fn every_profile_running_pairs_nothing_and_frees_every_proxy() {
    let plan = plan(
      &[running("p1"), running("p2")],
      &ids(&["x1", "x2"]),
      false,
      &HashSet::new(),
    );
    assert!(plan.pairs.is_empty());
    assert_eq!(plan.running_profile_ids, ids(&["p1", "p2"]));
    assert_eq!(plan.unused_proxy_ids, ids(&["x1", "x2"]));
  }

  #[test]
  fn a_profiles_own_proxy_is_not_treated_as_someone_elses() {
    // p1 already sits on x1. Because p1 is part of this distribution, x1 is
    // not "assigned elsewhere", so it stays in the pool and the run can
    // reshuffle it rather than refusing to touch it.
    let plan = plan(
      &[free("p1"), free("p2")],
      &ids(&["x1", "x2"]),
      false,
      &HashSet::new(),
    );
    assert!(plan.shared_proxy_ids.is_empty());
    assert_eq!(plan.pairs.len(), 2);
  }

  #[test]
  fn nothing_chosen_produces_an_empty_plan() {
    let plan = plan(&[], &[], false, &HashSet::new());
    assert_eq!(plan, DistributionPlan::default());
  }
}
