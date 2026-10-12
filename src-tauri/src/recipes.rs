//! Client for the cloud recipe routes. Storage, validation rules and replay
//! live in the cloud; this side only checks the shape before sending.

use crate::cloud_errors::{BackendFailure, FailureCodes};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const RECIPE_CODES: FailureCodes = FailureCodes {
  bad_request: "AGENT_RECIPE_INVALID",
  forbidden: "RECIPES_NOT_ENTITLED",
  not_found: "AGENT_RECIPE_NOT_FOUND",
  conflict: "AGENT_RECIPE_INVALID",
};

pub type RecipeStep = serde_json::Value;

const RECIPE_STEP_TYPES: [&str; 9] = [
  "navigate",
  "click",
  "type",
  "waitFor",
  "extract",
  "pressKey",
  "scroll",
  "screenshot",
  "sleep",
];

const RECIPE_TARGETED_TYPES: [&str; 4] = ["click", "type", "waitFor", "extract"];

const MAX_RECIPE_STEPS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecipe {
  pub id: String,
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub steps: Vec<RecipeStep>,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default)]
  pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RecipeList {
  Enveloped { recipes: Vec<AgentRecipe> },
  Bare(Vec<AgentRecipe>),
}

impl RecipeList {
  fn into_vec(self) -> Vec<AgentRecipe> {
    match self {
      Self::Enveloped { recipes } => recipes,
      Self::Bare(recipes) => recipes,
    }
  }
}

#[derive(Debug, Default, Deserialize)]
struct RecipeDeleted {
  #[serde(default)]
  deleted: Option<bool>,
}

fn base() -> String {
  format!("{}/api/agent/recipes", crate::cloud_auth::CLOUD_API_URL)
}

pub async fn list_recipes() -> Result<Vec<AgentRecipe>, String> {
  let list: RecipeList = request(reqwest::Method::GET, base(), None)
    .await
    .map_err(|e| failure("list", e))?;
  Ok(list.into_vec())
}

pub async fn create_recipe(name: &str, steps: &[RecipeStep]) -> Result<AgentRecipe, String> {
  let (name, steps) = validate_recipe(name, steps)?;
  request(
    reqwest::Method::POST,
    base(),
    Some(serde_json::json!({ "name": name, "steps": steps })),
  )
  .await
  .map_err(|e| failure("create", e))
}

pub async fn update_recipe(
  id: &str,
  name: &str,
  steps: &[RecipeStep],
) -> Result<AgentRecipe, String> {
  let (name, steps) = validate_recipe(name, steps)?;
  request(
    reqwest::Method::PATCH,
    format!("{}/{}", base(), urlencoding::encode(id)),
    Some(serde_json::json!({ "name": name, "steps": steps })),
  )
  .await
  .map_err(|e| failure("update", e))
}

pub async fn delete_recipe(id: &str) -> Result<bool, String> {
  let outcome: Option<RecipeDeleted> = request(
    reqwest::Method::DELETE,
    format!("{}/{}", base(), urlencoding::encode(id)),
    None,
  )
  .await
  .map_err(|e| failure("delete", e))?;
  Ok(recipe_deleted(outcome))
}

// A 204 with no body is a successful delete.
fn recipe_deleted(outcome: Option<RecipeDeleted>) -> bool {
  outcome.and_then(|reply| reply.deleted).unwrap_or(true)
}

fn validate_recipe(name: &str, steps: &[RecipeStep]) -> Result<(String, Vec<RecipeStep>), String> {
  let name = name.trim().to_string();
  if name.is_empty() {
    return Err(crate::backend_error("NAME_CANNOT_BE_EMPTY"));
  }
  if steps.is_empty() || steps.len() > MAX_RECIPE_STEPS {
    return Err(crate::backend_error("AGENT_RECIPE_INVALID"));
  }
  for step in steps {
    let Some(object) = step.as_object() else {
      return Err(crate::backend_error("AGENT_RECIPE_INVALID"));
    };
    let Some(kind) = object.get("type").and_then(serde_json::Value::as_str) else {
      return Err(crate::backend_error("AGENT_RECIPE_INVALID"));
    };
    if !RECIPE_STEP_TYPES.contains(&kind) {
      return Err(crate::backend_error("AGENT_RECIPE_INVALID"));
    }
    if RECIPE_TARGETED_TYPES.contains(&kind) {
      let has_selector = object
        .get("selector")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|selector| !selector.trim().is_empty());
      let has_locator = object
        .get("locator")
        .is_some_and(|locator| locator.as_object().is_some_and(|fields| !fields.is_empty()));
      if has_selector == has_locator {
        return Err(crate::backend_error("AGENT_RECIPE_INVALID"));
      }
    }
  }
  Ok((name, steps.to_vec()))
}

fn failure(action: &str, err: BackendFailure) -> String {
  log::warn!(
    "Recipe request failed action={action} code={} status={}",
    err.code,
    err.status
  );
  err.to_error_json()
}

async fn request<T: DeserializeOwned>(
  method: reqwest::Method,
  url: String,
  body: Option<serde_json::Value>,
) -> Result<T, BackendFailure> {
  crate::cloud_api::call(method, url, body, RECIPE_CODES).await
}

#[tauri::command]
pub async fn get_agent_recipes() -> Result<Vec<AgentRecipe>, String> {
  list_recipes().await
}

#[tauri::command]
pub async fn create_agent_recipe(
  name: String,
  steps: Vec<RecipeStep>,
) -> Result<AgentRecipe, String> {
  create_recipe(&name, &steps).await
}

#[tauri::command]
pub async fn update_agent_recipe(
  id: String,
  name: String,
  steps: Vec<RecipeStep>,
) -> Result<AgentRecipe, String> {
  update_recipe(&id, &name, &steps).await
}

#[tauri::command]
pub async fn delete_agent_recipe(id: String) -> Result<bool, String> {
  delete_recipe(&id).await
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cloud_api::decode_body;

  #[test]
  fn a_recipe_delete_with_no_body_is_a_success() {
    let empty: Option<RecipeDeleted> = decode_body(b"").expect("a 204 has no body");
    assert!(recipe_deleted(empty));
    let blank: Option<RecipeDeleted> = decode_body(b" \n").expect("whitespace is no body");
    assert!(recipe_deleted(blank));
    let refused: Option<RecipeDeleted> =
      decode_body(br#"{"deleted":false}"#).expect("an explicit answer");
    assert!(!recipe_deleted(refused));
    let bare: Option<RecipeDeleted> = decode_body(b"{}").expect("an empty object");
    assert!(recipe_deleted(bare));
  }

  #[test]
  fn an_empty_body_is_still_a_failure_where_a_value_is_required() {
    assert!(decode_body::<RecipeDeleted>(b"").is_err());
    assert!(decode_body::<AgentRecipe>(b"").is_err());
    assert!(decode_body::<Option<RecipeDeleted>>(b"<html>").is_err());
  }

  #[test]
  fn a_recipe_needs_a_name_and_steps_the_api_will_accept() {
    let navigate = serde_json::json!({"type": "navigate", "url": "https://example.com"});
    assert!(validate_recipe("  ", std::slice::from_ref(&navigate))
      .unwrap_err()
      .contains("NAME_CANNOT_BE_EMPTY"));
    assert!(validate_recipe("nightly", &[])
      .unwrap_err()
      .contains("AGENT_RECIPE_INVALID"));

    for rejected in [
      serde_json::json!("open the shop"),
      serde_json::json!({"url": "https://example.com"}),
      serde_json::json!({"type": "teleport", "url": "https://example.com"}),
      serde_json::json!({"type": "click"}),
      serde_json::json!({"type": "click", "selector": "#buy", "locator": {"role": "button"}}),
    ] {
      assert!(
        validate_recipe("nightly", std::slice::from_ref(&rejected))
          .unwrap_err()
          .contains("AGENT_RECIPE_INVALID"),
        "{rejected} must be refused"
      );
    }

    let click = serde_json::json!({"type": "click", "locator": {"role": "button", "name": "Buy"}});
    let (name, steps) = validate_recipe("  nightly  ", &[navigate.clone(), click.clone()]).unwrap();
    assert_eq!(name, "nightly");
    assert_eq!(steps, vec![navigate, click]);
  }

  #[test]
  fn the_recipe_list_decodes_bare_or_enveloped() {
    let bare: RecipeList =
      serde_json::from_str(r#"[{"id":"a","name":"A","steps":["x"]}]"#).unwrap();
    assert_eq!(bare.into_vec().len(), 1);
    let enveloped: RecipeList =
      serde_json::from_str(r#"{"recipes":[{"id":"a","name":"A","steps":["x"]}]}"#).unwrap();
    assert_eq!(enveloped.into_vec()[0].id, "a");
  }
}
