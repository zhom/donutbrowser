//! Turning a donutbrowser-infra HTTP failure into a stable, translatable code.
//!
//! Every cloud transport in this crate flattens its failures through
//! `api_call_with_retry`, which needs a `String` so it can sniff for a 401.
//! That flattening loses the status, and the body it carries is the backend's
//! own English — which would reach the user untranslated, the exact bug the
//! `{"code":…}` convention exists to prevent.
//!
//! So the backend sends a machine code, this module recovers it, and the
//! frontend resolves it through `translateBackendError`. When the backend
//! sends something else (a proxy error page, a gateway 502), the status alone
//! still picks a code the user can act on.

use serde_json::Value;
use std::collections::BTreeMap;

/// A backend failure reduced to the shape `translateBackendError` consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendFailure {
  /// The HTTP status it came from. 0 when the request never got that far.
  pub status: u16,
  pub code: String,
  pub params: BTreeMap<String, String>,
}

impl BackendFailure {
  /// Render as the `{"code":…,"params":{…}}` string a Tauri command returns.
  pub fn to_error_json(&self) -> String {
    let mut object = serde_json::Map::new();
    object.insert("code".to_string(), Value::String(self.code.clone()));
    if !self.params.is_empty() {
      let params = self
        .params
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect::<serde_json::Map<_, _>>();
      object.insert("params".to_string(), Value::Object(params));
    }
    Value::Object(object).to_string()
  }
}

/// Which code a status maps to when the body carries none.
///
/// 404 and 409 mean different things per route — "no schedule for this
/// profile" and "that run id is not yours" are both 404 — so each caller
/// supplies its own, rather than every route sharing one vague code.
#[derive(Debug, Clone, Copy)]
pub struct FailureCodes {
  pub bad_request: &'static str,
  pub forbidden: &'static str,
  pub not_found: &'static str,
  pub conflict: &'static str,
}

/// The desktop has no cloud session at all.
pub const NOT_SIGNED_IN: &str = "CLOUD_NOT_SIGNED_IN";
/// The request never reached donutbrowser-infra.
pub const UNREACHABLE: &str = "CLOUD_UNREACHABLE";
/// The backend answered, but with nothing the user can act on.
pub const UNAVAILABLE: &str = "CLOUD_REQUEST_FAILED";
/// Too many automation requests, backend side.
pub const RATE_LIMITED: &str = "REMOTE_RATE_LIMITED";
/// No host of the profile's OS has a free slot.
pub const NO_CAPACITY: &str = "REMOTE_NO_CAPACITY";

/// Recover `(status, body)` from the string `api_call_with_retry` hands back.
///
/// The transports encode a non-2xx as `"(503) no macos host free"` so the
/// helper can spot a 401 and still let the caller recover the kind. Anything
/// that is not that shape is a transport failure, not a status.
pub fn split_status(message: &str) -> Option<(u16, &str)> {
  let rest = message.strip_prefix('(')?;
  let (code, tail) = rest.split_once(')')?;
  let status = code.trim().parse::<u16>().ok()?;
  Some((status, tail.trim()))
}

/// Classify one HTTP failure.
pub fn classify(status: u16, body: &str, codes: FailureCodes) -> BackendFailure {
  if let Some(failure) = from_body(status, body) {
    return failure;
  }
  BackendFailure {
    status,
    code: code_for_status(status, codes).to_string(),
    params: BTreeMap::new(),
  }
}

/// Classify a flattened error string, whether or not it encodes a status.
///
/// Some callers strip the status before they get here (a typed error that
/// kept only the body), so a bare `{"code":…}` envelope is still recognised.
pub fn classify_message(message: &str, codes: FailureCodes) -> BackendFailure {
  if let Some((status, body)) = split_status(message) {
    return classify(status, body, codes);
  }
  if let Some(failure) = from_body(0, message) {
    return failure;
  }
  transport_failure(message)
}

/// A failure that never became an HTTP response.
///
/// `api_call_with_retry` reports a missing token as plain text, so the
/// signed-out case is recognised here rather than surfacing as "something went
/// wrong" — being signed out is a state the user can fix.
pub fn transport_failure(message: &str) -> BackendFailure {
  let code = if message.contains("Not logged in") || message.contains("No refresh token") {
    NOT_SIGNED_IN
  } else {
    UNREACHABLE
  };
  BackendFailure {
    status: 0,
    code: code.to_string(),
    params: BTreeMap::new(),
  }
}

fn code_for_status(status: u16, codes: FailureCodes) -> &'static str {
  match status {
    400 | 422 => codes.bad_request,
    401 => NOT_SIGNED_IN,
    402 | 403 => codes.forbidden,
    404 => codes.not_found,
    409 => codes.conflict,
    429 => RATE_LIMITED,
    503 => NO_CAPACITY,
    _ => UNAVAILABLE,
  }
}

/// Read the backend's own `{"code":…}` envelope when it sent one.
fn from_body(status: u16, body: &str) -> Option<BackendFailure> {
  let parsed = serde_json::from_str::<Value>(body).ok()?;
  let object = parsed.as_object()?;
  let code = object.get("code")?.as_str()?;
  if code.is_empty() {
    return None;
  }

  let mut params = BTreeMap::new();

  // The nested shape first, so a top-level key of the same name still wins.
  //
  // The cookie-bot routes send every interpolated value under `params`
  // (`{"code":…,"params":{…}}`) while the remote-session routes spread theirs
  // at the top level. Only the flat one was read, so
  // COOKIE_BOT_INVALID_TIMEZONE rendered with an empty timezone name,
  // COOKIE_BOT_SITE_LIMIT always showed the hardcoded fallback, and a team out
  // of hours was told it had "used 0 of 0".
  if let Some(Value::Object(nested)) = object.get("params") {
    collect_scalars(nested, &mut params);
  }

  for (key, value) in object {
    if key == "code" || key == "params" {
      continue;
    }
    if let Value::Array(items) = value {
      if key == "conflicts" {
        collect_conflict_params(items, &mut params);
      }
      continue;
    }
    if let Some(text) = scalar(value) {
      params.insert(key.clone(), text);
    }
  }

  Some(BackendFailure {
    status,
    code: code.to_string(),
    params,
  })
}

/// A JSON value that can be substituted into a translated sentence.
///
/// An object or an array has no rendering, so it is dropped rather than
/// stringified into the user's face.
fn scalar(value: &Value) -> Option<String> {
  match value {
    Value::String(text) => Some(text.clone()),
    Value::Number(number) => Some(number.to_string()),
    Value::Bool(flag) => Some(flag.to_string()),
    _ => None,
  }
}

fn collect_scalars(object: &serde_json::Map<String, Value>, params: &mut BTreeMap<String, String>) {
  for (key, value) in object {
    if let Some(text) = scalar(value) {
      params.insert(key.clone(), text);
    }
  }
}

/// Name the teammate whose enrolment blocks this one.
///
/// A schedule conflict is only actionable if the user learns WHO and WHEN, and
/// the list arrives as an array the generic scalar copy would drop. Only the
/// first entry is surfaced; the full list is in the response body for the UI.
fn collect_conflict_params(items: &[Value], params: &mut BTreeMap<String, String>) {
  params.insert("conflict_count".to_string(), items.len().to_string());
  let Some(first) = items.first().and_then(Value::as_object) else {
    return;
  };
  if let Some(email) = first.get("email").and_then(Value::as_str) {
    params.insert("email".to_string(), email.to_string());
  }
  if let Some(timezone) = first.get("timezone").and_then(Value::as_str) {
    params.insert("timezone".to_string(), timezone.to_string());
  }
  if let Some(minute) = first.get("run_at_minute").and_then(Value::as_u64) {
    params.insert("run_at_minute".to_string(), minute.to_string());
    params.insert("time".to_string(), format_minute_of_day(minute));
  }
}

/// Minute-of-day to a zero-padded 24h clock reading.
///
/// The value is a wall-clock offset in the conflicting enrolment's own
/// timezone, so there is no date and nothing to convert — only to render.
pub fn format_minute_of_day(minute: u64) -> String {
  let minute = minute % 1440;
  format!("{:02}:{:02}", minute / 60, minute % 60)
}

#[cfg(test)]
mod tests {
  use super::*;

  const CODES: FailureCodes = FailureCodes {
    bad_request: "BAD",
    forbidden: "FORBIDDEN",
    not_found: "MISSING",
    conflict: "CLASH",
  };

  #[test]
  fn the_backends_own_code_wins_over_the_status_default() {
    // The status table is a fallback for gateway pages. When infra names the
    // failure, that name is the one the user's locale has a string for.
    let failure = classify(403, r#"{"code":"COOKIE_BOT_NOT_ENTITLED"}"#, CODES);
    assert_eq!(failure.code, "COOKIE_BOT_NOT_ENTITLED");
    assert_eq!(failure.status, 403);
  }

  #[test]
  fn a_body_without_a_code_falls_back_to_the_routes_own_meaning() {
    // 404 means "no schedule" on one route and "no such run" on another;
    // sharing one code would tell the user the wrong thing on one of them.
    assert_eq!(classify(404, "Not Found", CODES).code, "MISSING");
    assert_eq!(classify(409, "", CODES).code, "CLASH");
    assert_eq!(classify(400, "<html>", CODES).code, "BAD");
  }

  #[test]
  fn capacity_and_rate_limits_are_never_reported_as_a_fault() {
    // 503 is "come back in a minute" — the fleet is four Windows hosts wide,
    // so a busy fleet is normal and must not look like an outage.
    assert_eq!(classify(503, "", CODES).code, NO_CAPACITY);
    assert_eq!(classify(429, "", CODES).code, RATE_LIMITED);
  }

  #[test]
  fn an_unauthenticated_response_is_always_the_signed_out_code() {
    // Never the route's forbidden code: "sign in" and "upgrade your plan" are
    // different instructions and the user can only follow one of them.
    assert_eq!(classify(401, "", CODES).code, NOT_SIGNED_IN);
    assert_eq!(classify(402, "", CODES).code, "FORBIDDEN");
  }

  #[test]
  fn scalar_body_fields_become_translation_params() {
    let failure = classify(
      403,
      r#"{"code":"REMOTE_HOURS_EXHAUSTED","granted":200,"used":201.5,"pooled":true}"#,
      CODES,
    );
    assert_eq!(
      failure.params.get("granted").map(String::as_str),
      Some("200")
    );
    assert_eq!(
      failure.params.get("used").map(String::as_str),
      Some("201.5")
    );
    assert_eq!(
      failure.params.get("pooled").map(String::as_str),
      Some("true")
    );
  }

  #[test]
  fn nested_params_are_read_because_that_is_the_shape_cookie_bot_sends() {
    // `body(code, params)` in cookie-bot.errors.ts returns `{code, params}`,
    // which Nest serialises verbatim. Reading only the top level dropped every
    // interpolated value: the timezone the user typed, the site limit, the
    // hours a team had actually spent.
    let failure = classify(
      400,
      r#"{"code":"COOKIE_BOT_INVALID_TIMEZONE","params":{"timezone":"Europe/Nowhere"}}"#,
      CODES,
    );
    assert_eq!(failure.code, "COOKIE_BOT_INVALID_TIMEZONE");
    assert_eq!(
      failure.params.get("timezone").map(String::as_str),
      Some("Europe/Nowhere")
    );

    let limit = classify(
      400,
      r#"{"code":"COOKIE_BOT_SITE_LIMIT","params":{"min":1,"max":40}}"#,
      CODES,
    );
    assert_eq!(limit.params.get("min").map(String::as_str), Some("1"));
    assert_eq!(limit.params.get("max").map(String::as_str), Some("40"));

    let hours = classify(
      403,
      r#"{"code":"REMOTE_HOURS_EXHAUSTED","params":{"granted":200,"used":214.5}}"#,
      CODES,
    );
    assert_eq!(hours.params.get("granted").map(String::as_str), Some("200"));
    assert_eq!(hours.params.get("used").map(String::as_str), Some("214.5"));
  }

  #[test]
  fn both_body_shapes_coexist_and_the_top_level_one_wins() {
    // The two planes disagree about where params live, and neither is going to
    // change for the other. A key present in both must resolve once.
    let failure = classify(
      403,
      r#"{"code":"REMOTE_HOURS_EXHAUSTED","granted":200,"params":{"granted":1,"used":5}}"#,
      CODES,
    );
    assert_eq!(
      failure.params.get("granted").map(String::as_str),
      Some("200")
    );
    assert_eq!(failure.params.get("used").map(String::as_str), Some("5"));
  }

  #[test]
  fn a_non_scalar_param_is_dropped_rather_than_rendered_as_json() {
    // These values are substituted into a translated sentence. An object has
    // no rendering, and `[object Object]` in a toast is worse than nothing.
    let failure = classify(
      400,
      r#"{"code":"COOKIE_BOT_INVALID_SCHEDULE","params":{"field":"sites","detail":{"a":1},"list":[1,2]}}"#,
      CODES,
    );
    assert_eq!(
      failure.params.get("field").map(String::as_str),
      Some("sites")
    );
    assert!(!failure.params.contains_key("detail"));
    assert!(!failure.params.contains_key("list"));
  }

  #[test]
  fn a_schedule_conflict_names_the_teammate_and_the_time() {
    // Without these the dialog can only say "someone else already warms this
    // profile", which is not something the user can act on.
    let failure = classify(
      409,
      r#"{"code":"COOKIE_BOT_SCHEDULE_CONFLICT","conflicts":[{"email":"alex@example.com","run_at_minute":120,"timezone":"Europe/Berlin"}]}"#,
      CODES,
    );
    assert_eq!(
      failure.params.get("email").map(String::as_str),
      Some("alex@example.com")
    );
    assert_eq!(
      failure.params.get("time").map(String::as_str),
      Some("02:00")
    );
    assert_eq!(
      failure.params.get("conflict_count").map(String::as_str),
      Some("1")
    );
  }

  #[test]
  fn minute_of_day_renders_as_a_padded_clock_reading() {
    assert_eq!(format_minute_of_day(0), "00:00");
    assert_eq!(format_minute_of_day(9 * 60 + 5), "09:05");
    assert_eq!(format_minute_of_day(1439), "23:59");
  }

  #[test]
  fn a_status_encoded_message_round_trips_to_its_code() {
    assert_eq!(
      classify_message(r#"(409) {"code":"COOKIE_BOT_RUN_IN_PROGRESS"}"#, CODES).code,
      "COOKIE_BOT_RUN_IN_PROGRESS"
    );
  }

  #[test]
  fn a_signed_out_desktop_is_told_to_sign_in_not_that_the_network_failed() {
    assert_eq!(classify_message("Not logged in", CODES).code, NOT_SIGNED_IN);
    assert_eq!(
      classify_message("reach backend: connection refused", CODES).code,
      UNREACHABLE
    );
  }

  #[test]
  fn the_rendered_json_is_what_translate_backend_error_parses() {
    let failure = classify(404, r#"{"code":"COOKIE_BOT_NOT_ENROLLED"}"#, CODES);
    assert_eq!(
      failure.to_error_json(),
      r#"{"code":"COOKIE_BOT_NOT_ENROLLED"}"#
    );

    let with_params = classify(
      403,
      r#"{"code":"REMOTE_HOURS_EXHAUSTED","granted":200}"#,
      CODES,
    );
    let parsed: Value = serde_json::from_str(&with_params.to_error_json())
      .expect("the rendered error must be valid JSON");
    assert_eq!(parsed["code"], "REMOTE_HOURS_EXHAUSTED");
    assert_eq!(parsed["params"]["granted"], "200");
  }

  #[test]
  fn split_status_does_not_misread_ordinary_prose() {
    assert_eq!(split_status("(503) busy"), Some((503, "busy")));
    assert_eq!(split_status("(nope) busy"), None);
    assert_eq!(split_status("decode response: expected value"), None);
  }
}
