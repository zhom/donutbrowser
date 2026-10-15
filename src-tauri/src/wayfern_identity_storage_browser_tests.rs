use super::*;
use std::path::PathBuf;

fn required(name: &str) -> String {
  std::env::var(name)
    .ok()
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| panic!("{name} is required"))
}

fn entitled(profile: &BrowserProfile) -> bool {
  profile.note.as_deref() == Some("entitled")
}

async fn session_for(profile: &BrowserProfile, token: Option<&str>) -> HeadlessWayfern {
  HeadlessWayfern::start(
    WayfernManager::instance(),
    profile,
    token,
    &format!("purpose=test profile={}", profile.id),
  )
  .await
  .unwrap_or_else(|e| panic!("the browser did not start: {e}"))
}

async fn answer(profile: &BrowserProfile, token: Option<&str>) -> Value {
  let config = profile.wayfern_config.clone().unwrap_or_default();
  let session = session_for(profile, token).await;
  let response = request(&session, conversion_params(profile, &config), false).await;
  session.stop().await;
  response.unwrap_or_else(|e| panic!("no answer for profile {}: {e:?}", profile.id))
}

/// Converts every stored profile in `DONUT_TEST_DATA_DIR`, which already holds
/// the browser build of each profile's version, and writes what the launch
/// sends for each converted profile to `<data dir>/conversion/<id>.json`.
/// Profiles whose note is `entitled` convert with `DONUT_TEST_WAYFERN_TOKEN`.
#[tokio::test(flavor = "current_thread")]
#[ignore = "needs a browser build in DONUT_TEST_DATA_DIR"]
async fn stored_profiles_convert_through_a_real_browser() {
  let root = PathBuf::from(required("DONUT_TEST_DATA_DIR"));
  std::env::set_var("DONUTBROWSER_DATA_DIR", &root);
  std::env::set_var("DONUTBROWSER_CACHE_DIR", root.join("cache"));
  let _data = crate::app_dirs::set_test_data_dir(root.clone());
  let _cache = crate::app_dirs::set_test_cache_dir(root.join("cache"));
  let token = std::env::var("DONUT_TEST_WAYFERN_TOKEN")
    .ok()
    .filter(|token| !token.is_empty());

  let stored: Vec<BrowserProfile> = ProfileManager::instance()
    .list_profiles()
    .expect("the stored profiles are readable")
    .into_iter()
    .filter(needs_conversion)
    .collect();
  assert!(!stored.is_empty(), "no stored profile needs conversion");
  let out = root.join("conversion");
  std::fs::create_dir_all(&out).unwrap();

  for before in stored {
    let token = if entitled(&before) {
      Some(
        token
          .as_deref()
          .expect("DONUT_TEST_WAYFERN_TOKEN is required for an entitled profile"),
      )
    } else {
      None
    };
    let first_answer = answer(&before, token).await;

    let converted = match token {
      Some(token) => {
        let session = session_for(&before, Some(token)).await;
        let result = convert_and_save(&session, &before, false, true).await;
        session.stop().await;
        result.unwrap_or_else(|e| panic!("profile {} did not convert: {e:?}", before.id))
      }
      None => convert_for_launch(&before)
        .await
        .unwrap_or_else(|e| panic!("profile {} did not launch: {e}", before.id)),
    };
    let config = converted.wayfern_config.clone().unwrap_or_default();
    assert_eq!(
      stored_shape(&config),
      StoredShape::Identity,
      "profile {} is stored as an identity",
      before.id
    );
    assert!(!needs_conversion(&converted));
    assert_eq!(
      config.identity_id.as_deref(),
      first_answer["identityId"].as_str(),
      "profile {} stores the identity the browser answered",
      before.id
    );

    let on_disk = read_profile(&before.id).expect("the converted profile is on disk");
    assert_eq!(config_value(&on_disk), config_value(&converted));
    assert!(crate::app_dirs::data_subdir()
      .join("wayfern-config-backups")
      .join(format!("{}.json", before.id))
      .is_file());

    let again = convert_for_launch(&on_disk)
      .await
      .expect("a converted profile launches");
    assert_eq!(
      config_value(&again),
      config_value(&on_disk),
      "a converted profile is never converted again"
    );

    let replay = answer(&before, token).await;
    assert_eq!(
      replay["identityId"], first_answer["identityId"],
      "the same stored profile answers with the same identity"
    );

    let document = WayfernManager::launch_identity_document(&config, &on_disk.native_os())
      .unwrap_or_else(|| panic!("profile {} has no launch document", before.id));
    let record = json!({
      "profileId": before.id.to_string(),
      "name": before.name,
      "entitled": token.is_some(),
      "answer": first_answer,
      "stored": serde_json::to_value(&on_disk.wayfern_config).unwrap(),
      "document": document,
    });
    std::fs::write(
      out.join(format!("{}.json", before.id)),
      serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
  }
}

/// Moves every stored Wayfern profile in `DONUT_TEST_DATA_DIR` to the build in
/// `binaries/wayfern/<DONUT_TEST_TARGET_VERSION>` the way the updater moves a
/// closed profile, converts the ones that need it through that browser, and
/// writes what the launch sends for each profile to
/// `<data dir>/conversion/<id>.json`. With `DONUT_TEST_WAYFERN_TOKEN` set the
/// conversion is entitled; without it the launch path converts unentitled.
#[tokio::test(flavor = "current_thread")]
#[ignore = "needs a browser build in DONUT_TEST_DATA_DIR"]
async fn stored_profiles_follow_an_upgrade() {
  let root = PathBuf::from(required("DONUT_TEST_DATA_DIR"));
  let target = required("DONUT_TEST_TARGET_VERSION");
  std::env::set_var("DONUTBROWSER_DATA_DIR", &root);
  std::env::set_var("DONUTBROWSER_CACHE_DIR", root.join("cache"));
  let _data = crate::app_dirs::set_test_data_dir(root.clone());
  let _cache = crate::app_dirs::set_test_cache_dir(root.join("cache"));
  let token = std::env::var("DONUT_TEST_WAYFERN_TOKEN")
    .ok()
    .filter(|token| !token.is_empty());
  assert!(crate::wayfern_manager::supports_identity_api(&target));

  let stored: Vec<BrowserProfile> = ProfileManager::instance()
    .list_profiles()
    .expect("the stored profiles are readable")
    .into_iter()
    .filter(|profile| profile.browser == "wayfern")
    .collect();
  assert!(!stored.is_empty(), "no stored Wayfern profile");
  let out = root.join("conversion");
  std::fs::create_dir_all(&out).unwrap();

  for original in stored {
    let config_before = original.wayfern_config.clone().unwrap_or_default();
    let shape_before = stored_shape(&config_before);
    let payload_keys: Vec<String> = stored_payload(&config_before)
      .map(|p| p.keys().cloned().collect())
      .unwrap_or_default();
    let overrides_before: Vec<String> =
      WayfernManager::stored_object(config_before.identity_overrides.as_deref())
        .keys()
        .cloned()
        .collect();

    // ProfileManager::update_profile_version for a closed profile moving to a
    // build that speaks the identity API.
    let mut moved = original.clone();
    moved.version = target.clone();
    moved.release_type = "stable".to_string();
    ProfileManager::instance().save_profile(&moved).unwrap();
    let moved = read_profile(&moved.id).expect("the moved profile is on disk");

    let needed = needs_conversion(&moved);
    let first_answer = if needed {
      Some(answer(&moved, token.as_deref()).await)
    } else {
      None
    };
    let converted = match (needed, token.as_deref()) {
      (true, Some(token)) => {
        let session = session_for(&moved, Some(token)).await;
        let result = convert_and_save(&session, &moved, false, true).await;
        session.stop().await;
        result.unwrap_or_else(|e| panic!("profile {} did not convert: {e:?}", moved.id))
      }
      _ => convert_for_launch(&moved)
        .await
        .unwrap_or_else(|e| panic!("profile {} did not launch: {e}", moved.id)),
    };
    let config = converted.wayfern_config.clone().unwrap_or_default();
    let on_disk = read_profile(&moved.id).expect("the converted profile is on disk");
    assert_eq!(config_value(&on_disk), config_value(&converted));
    assert!(
      !needs_conversion(&on_disk),
      "profile {} still needs conversion",
      moved.id
    );
    let again = convert_for_launch(&on_disk)
      .await
      .expect("a converted profile launches");
    assert_eq!(config_value(&again), config_value(&on_disk));
    if let Some(first) = &first_answer {
      assert_eq!(config.identity_id.as_deref(), first["identityId"].as_str());
      assert!(crate::app_dirs::data_subdir()
        .join("wayfern-config-backups")
        .join(format!("{}.json", moved.id))
        .is_file());
      let replay = answer(&moved, token.as_deref()).await;
      assert_eq!(replay["identityId"], first["identityId"]);
    }

    let document = WayfernManager::launch_identity_document(&config, &on_disk.native_os());
    let overrides_after: Vec<String> =
      WayfernManager::stored_object(config.identity_overrides.as_deref())
        .keys()
        .cloned()
        .collect();
    let record = json!({
      "profileId": moved.id.to_string(),
      "versionBefore": original.version,
      "versionAfter": on_disk.version,
      "shapeBefore": format!("{shape_before:?}"),
      "shapeAfter": format!("{:?}", stored_shape(&config)),
      "converted": needed,
      "entitled": token.is_some(),
      "identityBefore": config_before.identity_id,
      "osBefore": config_before.os,
      "payloadKeys": payload_keys,
      "overridesBefore": overrides_before,
      "overridesAfter": overrides_after,
      "answer": first_answer.map(|a| json!({
        "identityId": a["identityId"], "operatingSystem": a["operatingSystem"],
        "verdict": a["verdict"], "usable": a["usable"], "kept": a["kept"],
        "withheld": a["withheld"], "distance": a["distance"],
        "overrideKeys": a["overrides"].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
        "locationKeys": a["location"].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
      })),
      "stored": serde_json::to_value(&on_disk.wayfern_config).unwrap(),
      "document": document,
    });
    std::fs::write(
      out.join(format!("{}.json", moved.id)),
      serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
  }
}
