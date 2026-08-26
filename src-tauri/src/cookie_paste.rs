//! Parsing for the single "paste your cookies here" textarea.
//!
//! Whatever a user has on the clipboard is one of three shapes: a JSON export
//! (Cookie-Editor, CDP, Puppeteer, Playwright `storageState`, AdsPower), a
//! Netscape `cookies.txt` (curl, wget, yt-dlp, "Get cookies.txt"), or a raw
//! `Cookie:` / `Set-Cookie:` header copied out of DevTools. This module detects
//! which one it is and reports every repair and every rejection as a structured
//! issue, because the failure mode that actually hurts is a login cookie that
//! disappears without anyone being told.

use crate::cookie_manager::UnifiedCookie;
use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Largest expiry we accept as already being in seconds (roughly year 5000).
/// Anything above this is a millisecond timestamp that someone forgot to
/// convert, so it gets divided down rather than stored as a cookie that never
/// expires.
const MAX_PLAUSIBLE_EXPIRY_SECS: i64 = 95_617_584_000;

/// How many times the unit sniffer will divide by 1000. Two steps covers the
/// millisecond and microsecond epochs; a third is pure paranoia.
const MAX_UNIT_SNIFF_STEPS: u32 = 3;

/// Ceiling for an expiry as it was written, before the unit sniffer decides
/// whether it is seconds, milliseconds or microseconds. It leaves room for a
/// microsecond epoch while keeping every parsed expiry far from the ends of the
/// i64 range, where an `f64` cast saturates to `i64::MIN` and arithmetic on the
/// result stops behaving.
const MAX_RAW_EXPIRY: i64 = MAX_PLAUSIBLE_EXPIRY_SECS * 1_000_000;

/// RFC 2616 separators, plus HTAB. SP and the CTLs are handled separately.
const TOKEN_SEPARATORS: &str = "()<>@,;:\\\"/[]?={}\t";

/// Names that mean "attribute" rather than "cookie" inside a `Set-Cookie`
/// line. Only consulted once a real name=value pair has been seen on the same
/// line, so a paste that starts with `domain=...` stays a cookie named
/// `domain`.
const SET_COOKIE_ATTRIBUTES: [&str; 11] = [
  "domain",
  "path",
  "expires",
  "max-age",
  "samesite",
  "secure",
  "httponly",
  "partitioned",
  "priority",
  "version",
  "comment",
];

/// The attributes whose value we read and act on. Everything else in
/// `SET_COOKIE_ATTRIBUTES` has its value discarded, so a `name=value` pair that
/// lands on one of those has to be reported rather than vanish.
const ATTRIBUTES_WITH_A_USED_VALUE: [&str; 5] =
  ["domain", "path", "expires", "max-age", "samesite"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueSeverity {
  Error,
  Warning,
  Info,
}

/// One thing that happened to the paste. `code` is a stable SCREAMING_SNAKE
/// identifier the frontend maps to a translated string; `source` locates it
/// ("line 4", "cookie 12") and `params` carries the substitutions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieIssue {
  pub code: String,
  pub severity: IssueSeverity,
  pub source: Option<String>,
  pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteFormat {
  Json,
  Netscape,
  NameValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPaste {
  pub format: Option<PasteFormat>,
  pub cookies: Vec<UnifiedCookie>,
  pub issues: Vec<CookieIssue>,
  /// True when the paste is a bare `name=value` list and no site was supplied,
  /// so there is nothing to attach the cookies to. The caller asks for a site
  /// and parses again.
  pub site_required: bool,
  pub expired_count: usize,
}

fn issue(
  code: &str,
  severity: IssueSeverity,
  source: Option<&str>,
  params: &[(&str, &str)],
) -> CookieIssue {
  CookieIssue {
    code: code.to_string(),
    severity,
    source: source.map(str::to_string),
    params: params
      .iter()
      .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
      .collect(),
  }
}

/// The site the user typed next to the textarea, split into the parts the
/// parsers need.
struct SiteHint {
  host: String,
  /// Only an explicit `https://` marks the origin as secure. A bare hostname
  /// says nothing about the scheme, so we do not invent one.
  secure: bool,
}

/// A cookie as one of the format parsers understood it, before the shared
/// normalisation runs. Fields that the source format could not express are
/// `None` so `finalize` can tell "absent" from "explicitly set".
struct CookieDraft {
  name: String,
  value: String,
  /// `None` falls back to the pasted site.
  domain: Option<String>,
  /// Explicit host-only intent, from JSON `hostOnly` or the Netscape
  /// include-subdomains column. `None` leaves the leading dot to decide.
  host_only: Option<bool>,
  path: Option<String>,
  /// `None` is a session cookie.
  expires: Option<i64>,
  is_secure: bool,
  is_http_only: bool,
  same_site: i32,
  source: String,
}

fn now_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}

fn strip_bom(input: &str) -> &str {
  input.strip_prefix('\u{feff}').unwrap_or(input)
}

fn is_token(s: &str) -> bool {
  !s.is_empty()
    && s
      .chars()
      .all(|c| !c.is_control() && c != ' ' && !TOKEN_SEPARATORS.contains(c))
}

/// RFC 6265 cookie-value: no CTLs (which covers HTAB, LF and CR) and no
/// semicolon, since a semicolon would re-split the pair on the way back out.
fn is_valid_value(s: &str) -> bool {
  s.chars().all(|c| !c.is_control() && c != ';')
}

fn parse_site(raw: &str) -> Option<SiteHint> {
  let raw = raw.trim();
  if raw.is_empty() {
    return None;
  }
  let (secure, rest) = match raw.split_once("://") {
    Some((scheme, rest)) => (scheme.eq_ignore_ascii_case("https"), rest),
    None => (false, raw),
  };
  let rest = rest.split(['/', '?', '#']).next().unwrap_or("");
  // Drop userinfo and any port; a cookie domain carries neither.
  let rest = rest.rsplit('@').next().unwrap_or(rest);
  let host = match rest.rsplit_once(':') {
    Some((head, tail)) if !head.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head,
    _ => rest,
  };
  let host = normalize_domain(host)?;
  Some(SiteHint { host, secure })
}

/// Lowercase, drop the trailing root dot, and reject anything that is not a
/// hostname. The leading dot survives: it is how the Netscape and JSON formats
/// spell "include subdomains".
fn normalize_domain(raw: &str) -> Option<String> {
  let mut domain = raw.trim().to_ascii_lowercase();
  while domain.len() > 1 && domain.ends_with('.') {
    domain.pop();
  }
  if domain.is_empty() || domain == "." {
    return None;
  }
  let bare = domain.strip_prefix('.').unwrap_or(&domain);
  if bare.is_empty() {
    return None;
  }
  let rejected = |c: char| c.is_control() || c.is_whitespace() || "/\\?#@;,:[]{}<>\"'".contains(c);
  if bare.chars().any(rejected) {
    return None;
  }
  Some(domain)
}

fn same_site_from_token(raw: &str) -> Option<i32> {
  match raw.trim().to_ascii_lowercase().as_str() {
    "strict" => Some(2),
    "lax" => Some(1),
    "none" | "no_restriction" | "norestriction" => Some(0),
    "unspecified" | "default" | "null" | "" => Some(-1),
    other => other.parse::<i32>().ok().filter(|n| (-1..=2).contains(n)),
  }
}

/// Accepts the shapes a `Set-Cookie` `Expires` attribute actually arrives in,
/// plus a bare unix timestamp for the people who paste one.
fn parse_http_date(raw: &str) -> Option<i64> {
  let trimmed = raw.trim().trim_matches('"').trim();
  if trimmed.is_empty() {
    return None;
  }
  if let Ok(n) = trimmed.parse::<i64>() {
    return Some(n.clamp(-MAX_RAW_EXPIRY, MAX_RAW_EXPIRY));
  }
  if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
    return Some(dt.timestamp());
  }
  if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
    return Some(dt.timestamp());
  }
  const FORMATS: [&str; 7] = [
    "%a, %d %b %Y %H:%M:%S GMT",
    "%a, %d-%b-%Y %H:%M:%S GMT",
    "%A, %d-%b-%y %H:%M:%S GMT",
    "%a, %d %b %Y %H:%M:%S UTC",
    "%a, %d %b %Y %H:%M:%S",
    "%a %b %e %H:%M:%S %Y",
    "%Y-%m-%d %H:%M:%S",
  ];
  for format in FORMATS {
    if let Ok(dt) = NaiveDateTime::parse_from_str(trimmed, format) {
      return Some(dt.and_utc().timestamp());
    }
  }
  None
}

/// Pull the cookie array out of whichever JSON wrapper the exporter used.
fn json_cookie_array(value: &Value) -> Option<Vec<Value>> {
  match value {
    Value::Array(items) => Some(items.clone()),
    Value::Object(map) => {
      for key in ["cookies", "Cookies"] {
        if let Some(Value::Array(items)) = map.get(key) {
          return Some(items.clone());
        }
      }
      // A single cookie object, which is what copying one row out of DevTools
      // gives you.
      if map.get("name").and_then(Value::as_str).is_some() {
        return Some(vec![value.clone()]);
      }
      None
    }
    _ => None,
  }
}

/// A `cookies.txt` row is six or seven tab-separated fields carrying a
/// TRUE/FALSE column among the first few — the same columns `parse_netscape`
/// keys off. Demanding the whole shape rather than the mere presence of a tab
/// keeps one stray tab in a wrapped `Cookie:` header from routing the entire
/// paste into the Netscape parser.
fn is_netscape_row(line: &str) -> bool {
  let fields: Vec<&str> = line.split('\t').collect();
  if fields.len() != 6 && fields.len() != 7 {
    return false;
  }
  fields[1..=3].iter().any(|field| {
    let field = field.trim();
    field.eq_ignore_ascii_case("TRUE") || field.eq_ignore_ascii_case("FALSE")
  })
}

pub fn detect_format(input: &str) -> Option<PasteFormat> {
  let trimmed = strip_bom(input).trim();
  if trimmed.is_empty() {
    return None;
  }

  if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
    if json_cookie_array(&value).is_some() {
      return Some(PasteFormat::Json);
    }
  }
  // Broken JSON is still JSON, and a truncated clipboard is the commonest paste
  // failure there is. Routing it to the JSON parser turns "we do not recognise
  // this" into serde naming the line and column that went wrong.
  if trimmed.starts_with('[') || trimmed.starts_with('{') {
    return Some(PasteFormat::Json);
  }

  let mut seen_content = false;
  for raw in trimmed.lines() {
    let line = raw.trim_end_matches('\r').trim_start();
    if line.starts_with("#HttpOnly_") {
      return Some(PasteFormat::Netscape);
    }
    if !seen_content
      && (line.starts_with("# Netscape HTTP Cookie File") || line.starts_with("# HTTP Cookie File"))
    {
      return Some(PasteFormat::Netscape);
    }
    if line.starts_with('#') {
      continue;
    }
    if line.trim_end().is_empty() {
      continue;
    }
    seen_content = true;
    if is_netscape_row(line) {
      return Some(PasteFormat::Netscape);
    }
  }

  let has_pair = trimmed.lines().any(|raw| {
    let line = raw.trim();
    !line.is_empty() && line.contains('=')
  });
  if has_pair {
    return Some(PasteFormat::NameValue);
  }

  None
}

pub fn parse_paste(input: &str, site: Option<&str>) -> ParsedPaste {
  let mut issues: Vec<CookieIssue> = Vec::new();
  let cleaned = strip_bom(input);
  let now = now_secs();

  if cleaned.trim().is_empty() {
    issues.push(issue("EMPTY_INPUT", IssueSeverity::Error, None, &[]));
    return ParsedPaste {
      format: None,
      cookies: Vec::new(),
      issues,
      site_required: false,
      expired_count: 0,
    };
  }

  let hint = site.and_then(parse_site);
  if let Some(raw) = site {
    if !raw.trim().is_empty() && hint.is_none() {
      issues.push(issue(
        "SITE_INVALID",
        IssueSeverity::Warning,
        None,
        &[("site", raw.trim())],
      ));
    }
  }

  let format = match detect_format(cleaned) {
    Some(format) => format,
    None => {
      issues.push(issue(
        "UNRECOGNIZED_FORMAT",
        IssueSeverity::Error,
        None,
        &[],
      ));
      return ParsedPaste {
        format: None,
        cookies: Vec::new(),
        issues,
        site_required: false,
        expired_count: 0,
      };
    }
  };

  let mut site_required = false;
  let drafts = match format {
    PasteFormat::Json => parse_json(cleaned, &mut issues),
    PasteFormat::Netscape => parse_netscape(cleaned, &mut issues),
    PasteFormat::NameValue => match hint.as_ref() {
      Some(hint) => parse_name_value(cleaned, hint, now, &mut issues),
      None => {
        site_required = true;
        issues.push(issue("SITE_REQUIRED", IssueSeverity::Error, None, &[]));
        Vec::new()
      }
    },
  };

  let mut finalized: Vec<(UnifiedCookie, String)> = Vec::new();
  for draft in drafts {
    let source = draft.source.clone();
    if let Some(cookie) = finalize(draft, hint.as_ref(), now, &mut issues) {
      finalized.push((cookie, source));
    }
  }

  let cookies = dedupe(finalized, &mut issues);
  // A paste that yields nothing has to say why. A header-only cookies.txt or a
  // JSON array of one unusable entry would otherwise come back as a clean,
  // empty, unexplained success.
  if cookies.is_empty()
    && !site_required
    && !issues
      .iter()
      .any(|i| matches!(i.severity, IssueSeverity::Error))
  {
    issues.push(issue("NO_COOKIES_FOUND", IssueSeverity::Error, None, &[]));
  }

  let expired_count = cookies
    .iter()
    .filter(|c| c.expires != 0 && c.expires <= now)
    .count();

  ParsedPaste {
    format: Some(format),
    cookies,
    issues,
    site_required,
    expired_count,
  }
}

/// Shared normalisation. Everything that is true of a cookie regardless of the
/// format it was pasted in happens here, in one place, so the three parsers
/// cannot drift apart.
fn finalize(
  draft: CookieDraft,
  site: Option<&SiteHint>,
  now: i64,
  issues: &mut Vec<CookieIssue>,
) -> Option<UnifiedCookie> {
  let source = draft.source.as_str();

  let name = draft.name.trim().to_string();
  if name.is_empty() {
    issues.push(issue("NAME_EMPTY", IssueSeverity::Error, Some(source), &[]));
    return None;
  }
  if !is_token(&name) {
    issues.push(issue(
      "NAME_INVALID",
      IssueSeverity::Error,
      Some(source),
      &[("name", &name)],
    ));
    return None;
  }
  if !is_valid_value(&draft.value) {
    issues.push(issue(
      "VALUE_INVALID",
      IssueSeverity::Error,
      Some(source),
      &[("name", &name)],
    ));
    return None;
  }

  let raw_domain = match draft
    .domain
    .as_deref()
    .map(str::trim)
    .filter(|d| !d.is_empty())
  {
    Some(domain) => domain.to_string(),
    None => match site {
      Some(hint) => {
        issues.push(issue(
          "DOMAIN_FROM_SITE",
          IssueSeverity::Info,
          Some(source),
          &[("name", &name), ("domain", &hint.host)],
        ));
        hint.host.clone()
      }
      None => {
        issues.push(issue(
          "DOMAIN_MISSING",
          IssueSeverity::Error,
          Some(source),
          &[("name", &name)],
        ));
        return None;
      }
    },
  };
  let mut domain = match normalize_domain(&raw_domain) {
    Some(domain) => domain,
    None => {
      issues.push(issue(
        "DOMAIN_INVALID",
        IssueSeverity::Error,
        Some(source),
        &[("name", &name), ("domain", raw_domain.trim())],
      ));
      return None;
    }
  };

  // The leading dot and the host-only flag say the same thing twice, and
  // exporters disagree about which one is authoritative. The explicit flag
  // wins, and the correction is reported rather than applied in silence.
  match draft.host_only {
    Some(false) if !domain.starts_with('.') => {
      issues.push(issue(
        "HOST_ONLY_MISMATCH",
        IssueSeverity::Warning,
        Some(source),
        &[("name", &name), ("domain", &domain), ("hostOnly", "false")],
      ));
      domain.insert(0, '.');
    }
    Some(true) if domain.starts_with('.') => {
      issues.push(issue(
        "HOST_ONLY_MISMATCH",
        IssueSeverity::Warning,
        Some(source),
        &[("name", &name), ("domain", &domain), ("hostOnly", "true")],
      ));
      domain.remove(0);
    }
    _ => {}
  }

  let path = match draft
    .path
    .as_deref()
    .map(str::trim)
    .filter(|p| !p.is_empty())
  {
    Some(path) if path.starts_with('/') && is_valid_value(path) => path.to_string(),
    Some(path) => {
      issues.push(issue(
        "PATH_REPAIRED",
        IssueSeverity::Warning,
        Some(source),
        &[("name", &name), ("path", path)],
      ));
      if is_valid_value(path) {
        format!("/{path}")
      } else {
        "/".to_string()
      }
    }
    None => "/".to_string(),
  };

  let expires = match draft.expires {
    None => 0,
    // Playwright spells a session cookie `"expires": -1`, and 0 is already the
    // session sentinel, so neither is worth complaining about. Taking every
    // non-positive expiry here also keeps the sniffer below off `i64::MIN`,
    // where negating to compare a magnitude overflows.
    Some(raw) if raw <= 0 => 0,
    Some(raw) => {
      let mut expiry = raw;
      let mut steps = 0;
      while expiry > MAX_PLAUSIBLE_EXPIRY_SECS && steps < MAX_UNIT_SNIFF_STEPS {
        expiry /= 1000;
        steps += 1;
      }
      if steps > 0 {
        issues.push(issue(
          "EXPIRY_MILLISECONDS",
          IssueSeverity::Warning,
          Some(source),
          &[("name", &name), ("expires", &raw.to_string())],
        ));
      }
      if expiry > MAX_PLAUSIBLE_EXPIRY_SECS {
        issues.push(issue(
          "EXPIRY_CLAMPED",
          IssueSeverity::Warning,
          Some(source),
          &[("name", &name), ("expires", &raw.to_string())],
        ));
        expiry = MAX_PLAUSIBLE_EXPIRY_SECS;
      }
      expiry
    }
  };

  let same_site = if (-1..=2).contains(&draft.same_site) {
    draft.same_site
  } else {
    -1
  };
  if same_site == 0 && !draft.is_secure {
    issues.push(issue(
      "SAME_SITE_NONE_INSECURE",
      IssueSeverity::Warning,
      Some(source),
      &[("name", &name), ("domain", &domain)],
    ));
  }

  Some(UnifiedCookie {
    name,
    value: draft.value,
    domain,
    path,
    expires,
    is_secure: draft.is_secure,
    is_http_only: draft.is_http_only,
    same_site,
    creation_time: now,
    last_accessed: now,
  })
}

/// A cookie store is keyed on (domain, name, path), so a paste that repeats one
/// is a self-overwrite. The last copy wins, matching what a browser would do
/// replaying the same set in order, and the shadowed one is reported.
fn dedupe(
  finalized: Vec<(UnifiedCookie, String)>,
  issues: &mut Vec<CookieIssue>,
) -> Vec<UnifiedCookie> {
  let mut seen: HashMap<(String, String, String), usize> = HashMap::new();
  let mut shadowed = vec![false; finalized.len()];

  for (index, (cookie, _)) in finalized.iter().enumerate() {
    let key = (
      cookie.domain.clone(),
      cookie.name.clone(),
      cookie.path.clone(),
    );
    if let Some(previous) = seen.insert(key, index) {
      shadowed[previous] = true;
      let (earlier, earlier_source) = &finalized[previous];
      issues.push(issue(
        "DUPLICATE_COOKIE",
        IssueSeverity::Warning,
        Some(earlier_source),
        &[
          ("name", &earlier.name),
          ("domain", &earlier.domain),
          ("path", &earlier.path),
        ],
      ));
    }
  }

  finalized
    .into_iter()
    .zip(shadowed)
    .filter(|(_, shadowed)| !shadowed)
    .map(|((cookie, _), _)| cookie)
    .collect()
}

fn read_bool(
  object: &serde_json::Map<String, Value>,
  keys: &[&str],
  source: &str,
  issues: &mut Vec<CookieIssue>,
) -> Option<bool> {
  let (key, value) = keys
    .iter()
    .find_map(|key| object.get(*key).filter(|v| !v.is_null()).map(|v| (*key, v)))?;

  match value {
    Value::Bool(b) => Some(*b),
    Value::String(s) => {
      let coerced = match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
      };
      match coerced {
        Some(b) => {
          issues.push(issue(
            "BOOL_COERCED_FROM_STRING",
            IssueSeverity::Warning,
            Some(source),
            &[("field", key), ("value", s)],
          ));
          Some(b)
        }
        None => {
          issues.push(issue(
            "BOOL_INVALID",
            IssueSeverity::Warning,
            Some(source),
            &[("field", key), ("value", s)],
          ));
          None
        }
      }
    }
    Value::Number(n) => match n.as_i64() {
      Some(0) => Some(false),
      Some(1) => Some(true),
      _ => {
        issues.push(issue(
          "BOOL_INVALID",
          IssueSeverity::Warning,
          Some(source),
          &[("field", key), ("value", &n.to_string())],
        ));
        None
      }
    },
    other => {
      issues.push(issue(
        "BOOL_INVALID",
        IssueSeverity::Warning,
        Some(source),
        &[("field", key), ("value", &other.to_string())],
      ));
      None
    }
  }
}

/// A float-to-integer cast in Rust saturates, so `-1e30` arrives as `i64::MIN`
/// and every later bound check on it is meaningless. Bounding the float first
/// keeps a nonsense expiry a nonsense expiry instead of an extreme the rest of
/// the module has to defend against.
fn clamp_raw_expiry(seconds: f64) -> i64 {
  let bound = MAX_RAW_EXPIRY as f64;
  seconds.trunc().clamp(-bound, bound) as i64
}

/// Returns `None` for a session cookie. `session: true` wins outright;
/// otherwise the first present expiry key decides, whichever dialect it came
/// from, because reading only `expirationDate` turned every CDP, Puppeteer,
/// Playwright and WebDriver export into session cookies.
fn read_expiry(
  object: &serde_json::Map<String, Value>,
  source: &str,
  issues: &mut Vec<CookieIssue>,
) -> Option<i64> {
  if read_bool(object, &["session"], source, issues) == Some(true) {
    return None;
  }

  let (key, value) = ["expirationDate", "expires", "expiry"]
    .iter()
    .find_map(|key| object.get(*key).filter(|v| !v.is_null()).map(|v| (*key, v)))?;

  match value {
    Value::Number(n) => n.as_f64().map(clamp_raw_expiry),
    Value::String(s) => match s.trim().parse::<f64>() {
      Ok(f) => Some(clamp_raw_expiry(f)),
      Err(_) => match parse_http_date(s) {
        Some(t) => Some(t),
        None => {
          issues.push(issue(
            "EXPIRY_INVALID",
            IssueSeverity::Warning,
            Some(source),
            &[("field", key), ("value", s.trim())],
          ));
          None
        }
      },
    },
    other => {
      issues.push(issue(
        "EXPIRY_INVALID",
        IssueSeverity::Warning,
        Some(source),
        &[("field", key), ("value", &other.to_string())],
      ));
      None
    }
  }
}

fn read_same_site(
  object: &serde_json::Map<String, Value>,
  source: &str,
  issues: &mut Vec<CookieIssue>,
) -> i32 {
  let Some(value) = ["sameSite", "samesite", "same_site"]
    .iter()
    .find_map(|key| object.get(*key).filter(|v| !v.is_null()))
  else {
    return -1;
  };

  let token = match value {
    Value::String(s) => s.clone(),
    Value::Number(n) => n.to_string(),
    other => other.to_string(),
  };
  match same_site_from_token(&token) {
    Some(resolved) => resolved,
    None => {
      issues.push(issue(
        "SAME_SITE_UNRECOGNIZED",
        IssueSeverity::Warning,
        Some(source),
        &[("value", token.trim())],
      ));
      -1
    }
  }
}

fn parse_json(input: &str, issues: &mut Vec<CookieIssue>) -> Vec<CookieDraft> {
  let value: Value = match serde_json::from_str(input.trim()) {
    Ok(value) => value,
    Err(e) => {
      issues.push(issue(
        "JSON_PARSE_FAILED",
        IssueSeverity::Error,
        None,
        &[("message", &e.to_string())],
      ));
      return Vec::new();
    }
  };
  let Some(items) = json_cookie_array(&value) else {
    issues.push(issue(
      "JSON_NOT_COOKIE_LIST",
      IssueSeverity::Error,
      None,
      &[],
    ));
    return Vec::new();
  };

  let mut drafts = Vec::new();
  for (index, item) in items.iter().enumerate() {
    let source = format!("cookie {}", index + 1);
    let Some(object) = item.as_object() else {
      issues.push(issue(
        "JSON_ENTRY_NOT_OBJECT",
        IssueSeverity::Error,
        Some(&source),
        &[],
      ));
      continue;
    };

    let name = match object.get("name") {
      None | Some(Value::Null) => {
        issues.push(issue(
          "NAME_MISSING",
          IssueSeverity::Error,
          Some(&source),
          &[],
        ));
        continue;
      }
      Some(Value::String(s)) => s.clone(),
      Some(other) => {
        issues.push(issue(
          "NAME_INVALID",
          IssueSeverity::Error,
          Some(&source),
          &[("name", &other.to_string())],
        ));
        continue;
      }
    };

    let value = match object.get("value") {
      None | Some(Value::Null) => String::new(),
      Some(Value::String(s)) => s.clone(),
      Some(other) => {
        // The name goes in the issue and the value never does: issue params are
        // rendered in the dialog and survive into the import result, and the
        // value IS the credential.
        issues.push(issue(
          "VALUE_COERCED",
          IssueSeverity::Warning,
          Some(&source),
          &[("name", &name)],
        ));
        other.to_string()
      }
    };

    let domain = match object.get("domain") {
      None | Some(Value::Null) => None,
      Some(Value::String(s)) => Some(s.clone()),
      Some(other) => {
        issues.push(issue(
          "DOMAIN_INVALID",
          IssueSeverity::Error,
          Some(&source),
          &[("name", &name), ("domain", &other.to_string())],
        ));
        continue;
      }
    };

    let path = object
      .get("path")
      .and_then(Value::as_str)
      .map(str::to_string);

    drafts.push(CookieDraft {
      name,
      value,
      domain,
      host_only: read_bool(
        object,
        &["hostOnly", "hostonly", "host_only"],
        &source,
        issues,
      ),
      path,
      expires: read_expiry(object, &source, issues),
      is_secure: read_bool(object, &["secure", "isSecure"], &source, issues).unwrap_or(false),
      is_http_only: read_bool(
        object,
        &["httpOnly", "httponly", "http_only", "isHttpOnly"],
        &source,
        issues,
      )
      .unwrap_or(false),
      same_site: read_same_site(object, &source, issues),
      source,
    });
  }

  drafts
}

fn parse_netscape(input: &str, issues: &mut Vec<CookieIssue>) -> Vec<CookieDraft> {
  let mut drafts = Vec::new();

  for (index, raw) in input.lines().enumerate() {
    let source = format!("line {}", index + 1);
    let line = raw.trim_end_matches('\r').trim_start();

    // This prefix has to be tested before the comment test, or every cookie
    // curl, wget, yt-dlp and the cookies.txt extensions mark HttpOnly — which
    // is to say every login cookie — is thrown away as a comment.
    let (body, prefixed_http_only) = match line.strip_prefix("#HttpOnly_") {
      Some(rest) => (rest, true),
      None => {
        if line.starts_with('#') || line.trim_end().is_empty() {
          continue;
        }
        (line, false)
      }
    };

    let mut fields: Vec<&str> = body.split('\t').collect();
    if fields.len() == 6
      && (fields[2].eq_ignore_ascii_case("TRUE") || fields[2].eq_ignore_ascii_case("FALSE"))
    {
      // curl omits the path column when it is the default one.
      fields.insert(2, "/");
      issues.push(issue(
        "NETSCAPE_PATH_OMITTED",
        IssueSeverity::Warning,
        Some(&source),
        &[],
      ));
    }
    if fields.len() != 7 {
      issues.push(issue(
        "NETSCAPE_FIELD_COUNT",
        IssueSeverity::Error,
        Some(&source),
        &[("expected", "7"), ("actual", &fields.len().to_string())],
      ));
      continue;
    }

    let host_only = if fields[1].eq_ignore_ascii_case("TRUE") {
      Some(false)
    } else if fields[1].eq_ignore_ascii_case("FALSE") {
      Some(true)
    } else {
      issues.push(issue(
        "NETSCAPE_INCLUDE_SUBDOMAINS_INVALID",
        IssueSeverity::Warning,
        Some(&source),
        &[("value", fields[1].trim())],
      ));
      None
    };

    let is_secure = if fields[3].eq_ignore_ascii_case("TRUE") {
      true
    } else if fields[3].eq_ignore_ascii_case("FALSE") {
      false
    } else {
      issues.push(issue(
        "NETSCAPE_SECURE_INVALID",
        IssueSeverity::Warning,
        Some(&source),
        &[("value", fields[3].trim())],
      ));
      false
    };

    let raw_expiry = fields[4].trim();
    let expires = if raw_expiry.is_empty() || raw_expiry == "0" {
      None
    } else {
      let (whole, fraction) = match raw_expiry.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (raw_expiry, None),
      };
      let numeric = !whole.is_empty()
        && whole.chars().all(|c| c.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()));
      if !numeric {
        // Garbage here used to become 0, which meant "session" — a live
        // credential conjured out of a malformed line, with nothing reported.
        issues.push(issue(
          "NETSCAPE_EXPIRY_INVALID",
          IssueSeverity::Error,
          Some(&source),
          &[("value", raw_expiry)],
        ));
        continue;
      }
      match whole.parse::<i64>() {
        Ok(seconds) => Some(seconds),
        Err(_) => {
          // All digits but wider than i64: a typo, not a timestamp. Pinning it
          // to the ceiling keeps the cookie without inventing an epoch.
          issues.push(issue(
            "EXPIRY_CLAMPED",
            IssueSeverity::Warning,
            Some(&source),
            &[("value", raw_expiry)],
          ));
          Some(MAX_PLAUSIBLE_EXPIRY_SECS)
        }
      }
    };

    drafts.push(CookieDraft {
      name: fields[5].to_string(),
      value: fields[6].to_string(),
      domain: Some(fields[0].to_string()),
      host_only,
      path: Some(fields[2].to_string()),
      expires,
      is_secure,
      is_http_only: prefixed_http_only,
      // The format has no column for it, so claiming anything else would be an
      // invention. -1 is Chromium's "unspecified".
      same_site: -1,
      source,
    });
  }

  drafts
}

fn parse_name_value(
  input: &str,
  site: &SiteHint,
  now: i64,
  issues: &mut Vec<CookieIssue>,
) -> Vec<CookieDraft> {
  let mut drafts = Vec::new();

  for (index, raw) in input.lines().enumerate() {
    let source = format!("line {}", index + 1);
    let mut line = raw.trim_end_matches('\r').trim();
    if line.is_empty() {
      continue;
    }
    let mut is_request_header = false;
    for prefix in ["set-cookie:", "cookie:"] {
      // Compared as bytes: `line` may start mid-way through a multi-byte
      // character, which slicing by the prefix length would split.
      let bytes = line.as_bytes();
      if bytes.len() >= prefix.len()
        && bytes[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
      {
        is_request_header = prefix == "cookie:";
        line = line[prefix.len()..].trim();
        break;
      }
    }
    if line.is_empty() {
      continue;
    }

    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut attributes: HashMap<String, String> = HashMap::new();

    for segment in line.split(';') {
      let segment = segment.trim();
      if segment.is_empty() {
        continue;
      }
      // Split at the FIRST '=' so a base64 value keeps its padding.
      let (raw_name, raw_value) = match segment.find('=') {
        Some(at) => (&segment[..at], &segment[at + 1..]),
        None => (segment, ""),
      };
      let name = raw_name.trim();
      let value = raw_value.trim();

      // A `Cookie:` request header carries no attributes at all, so a segment
      // named `priority` or `secure` there is a cookie with that name. Reading
      // it as a directive drops it and, worse, applies it to its neighbours.
      let is_attribute = !is_request_header
        && !pairs.is_empty()
        && SET_COOKIE_ATTRIBUTES
          .iter()
          .any(|attribute| name.eq_ignore_ascii_case(attribute));
      if is_attribute {
        let lowered = name.to_ascii_lowercase();
        // Nothing may disappear in silence: an attribute we act on shows up in
        // the previewed cookie, but one we only parse and drop would not.
        if !value.is_empty() && !ATTRIBUTES_WITH_A_USED_VALUE.contains(&lowered.as_str()) {
          issues.push(issue(
            "PAIR_TREATED_AS_ATTRIBUTE",
            IssueSeverity::Info,
            Some(&source),
            &[("name", name)],
          ));
        }
        attributes.insert(lowered, value.to_string());
        continue;
      }
      if name.is_empty() {
        issues.push(issue(
          "NAME_EMPTY",
          IssueSeverity::Error,
          Some(&source),
          &[],
        ));
        continue;
      }
      pairs.push((name.to_string(), value.to_string()));
    }

    if pairs.is_empty() {
      issues.push(issue(
        "NAME_VALUE_NO_PAIR",
        IssueSeverity::Warning,
        Some(&source),
        &[],
      ));
      continue;
    }

    if let Some(domain) = attributes.get("domain") {
      // Compared with the leading dot intact: `.example.com` and `example.com`
      // are different scopes, and dropping that difference is worth saying.
      let stated = domain.trim().to_ascii_lowercase();
      if !stated.is_empty() && stated != site.host {
        issues.push(issue(
          "DOMAIN_ATTRIBUTE_IGNORED",
          IssueSeverity::Info,
          Some(&source),
          &[("domain", domain.trim()), ("site", &site.host)],
        ));
      }
    }

    let mut expires = None;
    let mut deleted = false;
    if let Some(max_age) = attributes.get("max-age") {
      match max_age.trim().parse::<i64>() {
        Ok(seconds) if seconds > 0 => expires = Some(now.saturating_add(seconds)),
        // RFC 6265: a non-positive Max-Age deletes the cookie. Importing a
        // deletion would be importing nothing at all.
        Ok(_) => deleted = true,
        Err(_) => issues.push(issue(
          "MAX_AGE_INVALID",
          IssueSeverity::Warning,
          Some(&source),
          &[("value", max_age.trim())],
        )),
      }
    }
    if deleted {
      for (name, _) in &pairs {
        issues.push(issue(
          "MAX_AGE_DELETION",
          IssueSeverity::Warning,
          Some(&source),
          &[("name", name)],
        ));
      }
      continue;
    }
    if expires.is_none() {
      if let Some(raw_expires) = attributes.get("expires") {
        match parse_http_date(raw_expires) {
          Some(timestamp) => expires = Some(timestamp),
          None => issues.push(issue(
            "EXPIRES_INVALID",
            IssueSeverity::Warning,
            Some(&source),
            &[("value", raw_expires.trim())],
          )),
        }
      }
    }

    let path = attributes.get("path").cloned();
    let is_secure = attributes.contains_key("secure") || site.secure;
    let is_http_only = attributes.contains_key("httponly");
    let same_site = match attributes.get("samesite") {
      Some(raw_same_site) => match same_site_from_token(raw_same_site) {
        Some(resolved) => resolved,
        None => {
          issues.push(issue(
            "SAME_SITE_UNRECOGNIZED",
            IssueSeverity::Warning,
            Some(&source),
            &[("value", raw_same_site.trim())],
          ));
          -1
        }
      },
      None => -1,
    };

    for (name, value) in pairs {
      if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        // RFC 6265 4.1.1 makes the quotes part of the value, which is almost
        // never what someone pasting a header meant.
        issues.push(issue(
          "QUOTED_VALUE",
          IssueSeverity::Info,
          Some(&source),
          &[("name", &name)],
        ));
      }
      drafts.push(CookieDraft {
        name,
        value,
        // There is no domain to infer from a bare pair, so the site the user
        // named is the only honest answer.
        domain: Some(site.host.clone()),
        host_only: None,
        path: path.clone(),
        expires,
        is_secure,
        is_http_only,
        same_site,
        source: source.clone(),
      });
    }
  }

  drafts
}

#[cfg(test)]
mod tests {
  use super::*;

  fn tsv(fields: &[&str]) -> String {
    fields.join("\t")
  }

  fn codes(parsed: &ParsedPaste) -> Vec<&str> {
    parsed.issues.iter().map(|i| i.code.as_str()).collect()
  }

  fn has_code(parsed: &ParsedPaste, code: &str) -> bool {
    parsed.issues.iter().any(|i| i.code == code)
  }

  fn cookie<'a>(parsed: &'a ParsedPaste, name: &str) -> &'a UnifiedCookie {
    parsed
      .cookies
      .iter()
      .find(|c| c.name == name)
      .unwrap_or_else(|| panic!("no cookie named {name}, issues: {:?}", codes(parsed)))
  }

  #[test]
  fn test_detect_format_is_deterministic_per_shape() {
    assert_eq!(detect_format("[]"), Some(PasteFormat::Json));
    assert_eq!(
      detect_format(r#"{"cookies":[],"origins":[]}"#),
      Some(PasteFormat::Json)
    );
    assert_eq!(
      detect_format(r#"{"name":"sid","value":"1","domain":"a.com"}"#),
      Some(PasteFormat::Json)
    );
    assert_eq!(
      detect_format(&tsv(&[".a.com", "TRUE", "/", "FALSE", "0", "sid", "1"])),
      Some(PasteFormat::Netscape)
    );
    assert_eq!(
      detect_format("# Netscape HTTP Cookie File\n"),
      Some(PasteFormat::Netscape)
    );
    assert_eq!(
      detect_format("sid=1; other=2"),
      Some(PasteFormat::NameValue)
    );
    assert_eq!(detect_format(""), None);
    assert_eq!(detect_format("   \n\t\n  "), None);
    assert_eq!(detect_format("just some prose"), None);
    // A comment-only header still reads as Netscape, so the paste is reported
    // as an empty cookies.txt rather than as an unknown format.
    assert_eq!(
      detect_format("# HTTP Cookie File\n# nothing else\n"),
      Some(PasteFormat::Netscape)
    );
    // A BOM in front of a JSON export must not change the answer.
    assert_eq!(detect_format("\u{feff}[]"), Some(PasteFormat::Json));
  }

  #[test]
  fn test_http_only_prefixed_lines_survive() {
    // Exactly what `curl -c` writes for a login session.
    let paste = format!(
      "# Netscape HTTP Cookie File\n# This file was generated by libcurl!\n{}\n{}\n{}\n",
      tsv(&[
        "#HttpOnly_.example.com",
        "TRUE",
        "/",
        "TRUE",
        "2145916800",
        "session_id",
        "abc123"
      ]),
      tsv(&[
        "#HttpOnly_www.example.com",
        "FALSE",
        "/app",
        "TRUE",
        "2145916800",
        "csrf",
        "tok"
      ]),
      tsv(&[
        ".example.com",
        "TRUE",
        "/",
        "FALSE",
        "2145916800",
        "theme",
        "dark"
      ]),
    );

    let parsed = parse_paste(&paste, None);
    assert_eq!(parsed.format, Some(PasteFormat::Netscape));
    assert_eq!(parsed.cookies.len(), 3, "issues: {:?}", codes(&parsed));

    let session = cookie(&parsed, "session_id");
    assert!(session.is_http_only);
    assert!(session.is_secure);
    assert_eq!(session.domain, ".example.com");
    assert_eq!(session.expires, 2_145_916_800);
    assert_eq!(session.same_site, -1);

    let csrf = cookie(&parsed, "csrf");
    assert!(csrf.is_http_only);
    assert_eq!(csrf.domain, "www.example.com");
    assert_eq!(csrf.path, "/app");

    assert!(!cookie(&parsed, "theme").is_http_only);
    assert!(!has_code(&parsed, "NETSCAPE_FIELD_COUNT"));
  }

  #[test]
  fn test_netscape_six_field_curl_tolerance() {
    let paste = tsv(&[".example.com", "TRUE", "FALSE", "2145916800", "sid", "v"]);
    let parsed = parse_paste(&paste, None);

    assert_eq!(parsed.cookies.len(), 1, "issues: {:?}", codes(&parsed));
    let sid = cookie(&parsed, "sid");
    assert_eq!(sid.path, "/");
    assert!(!sid.is_secure);
    assert_eq!(sid.expires, 2_145_916_800);
    assert!(has_code(&parsed, "NETSCAPE_PATH_OMITTED"));
  }

  #[test]
  fn test_netscape_wrong_field_count_is_dropped_and_reported() {
    let paste = format!(
      "{}\n{}\n",
      tsv(&[".a.com", "TRUE", "/", "FALSE", "0"]),
      tsv(&[".a.com", "TRUE", "/", "FALSE", "0", "ok", "1"]),
    );
    let parsed = parse_paste(&paste, None);

    assert_eq!(parsed.cookies.len(), 1);
    let reported = parsed
      .issues
      .iter()
      .find(|i| i.code == "NETSCAPE_FIELD_COUNT")
      .expect("field count issue");
    assert_eq!(reported.source.as_deref(), Some("line 1"));
    assert_eq!(reported.params.get("actual").map(String::as_str), Some("5"));
  }

  #[test]
  fn test_non_numeric_expiry_is_dropped_not_turned_into_a_live_cookie() {
    let paste = format!(
      "{}\n{}\n",
      tsv(&[
        ".example.com",
        "TRUE",
        "/",
        "FALSE",
        "not-a-number",
        "auth",
        "secret"
      ]),
      tsv(&[".example.com", "TRUE", "/", "FALSE", "0", "keep", "1"]),
    );
    let parsed = parse_paste(&paste, None);

    assert!(
      parsed.cookies.iter().all(|c| c.name != "auth"),
      "a malformed expiry must never become a session cookie"
    );
    assert_eq!(parsed.cookies.len(), 1);
    assert_eq!(cookie(&parsed, "keep").expires, 0);
    let reported = parsed
      .issues
      .iter()
      .find(|i| i.code == "NETSCAPE_EXPIRY_INVALID")
      .expect("expiry issue");
    assert_eq!(reported.source.as_deref(), Some("line 1"));
  }

  #[test]
  fn test_netscape_fractional_expiry_truncates_and_empty_name_drops() {
    let paste = format!(
      "{}\n{}\n",
      tsv(&[".a.com", "TRUE", "/", "FALSE", "2145916800.75", "sid", "v"]),
      tsv(&[".a.com", "TRUE", "/", "FALSE", "0", "", "orphan"]),
    );
    let parsed = parse_paste(&paste, None);

    assert_eq!(parsed.cookies.len(), 1);
    assert_eq!(cookie(&parsed, "sid").expires, 2_145_916_800);
    assert!(has_code(&parsed, "NAME_EMPTY"));
  }

  #[test]
  fn test_playwright_storage_state_wrapper() {
    let paste = r#"{
      "cookies": [
        {
          "name": "sid",
          "value": "abc",
          "domain": ".example.com",
          "path": "/",
          "expires": -1,
          "httpOnly": true,
          "secure": true,
          "sameSite": "Lax"
        },
        {
          "name": "pref",
          "value": "x",
          "domain": "example.com",
          "path": "/",
          "expires": 2145916800,
          "httpOnly": false,
          "secure": false,
          "sameSite": "None"
        }
      ],
      "origins": []
    }"#;

    let parsed = parse_paste(paste, None);
    assert_eq!(parsed.format, Some(PasteFormat::Json));
    assert_eq!(parsed.cookies.len(), 2, "issues: {:?}", codes(&parsed));

    let sid = cookie(&parsed, "sid");
    assert_eq!(sid.expires, 0, "expires:-1 is Playwright for 'session'");
    assert_eq!(sid.same_site, 1);
    assert!(sid.is_http_only && sid.is_secure);

    let pref = cookie(&parsed, "pref");
    assert_eq!(pref.expires, 2_145_916_800);
    assert_eq!(pref.same_site, 0);
    // SameSite=None without Secure is a cookie Chromium will refuse to send.
    assert!(has_code(&parsed, "SAME_SITE_NONE_INSECURE"));
  }

  #[test]
  fn test_cdp_dump_with_capitalised_same_site() {
    let paste = r#"[
      {"name":"a","value":"1","domain":".example.com","path":"/","expires":2145916800,"size":8,"httpOnly":true,"secure":true,"session":false,"sameSite":"Strict"},
      {"name":"b","value":"2","domain":".example.com","path":"/","expires":2145916800,"httpOnly":false,"secure":true,"session":false,"sameSite":"Lax"},
      {"name":"c","value":"3","domain":".example.com","path":"/","expires":-1,"httpOnly":false,"secure":true,"session":true,"sameSite":"None"}
    ]"#;

    let parsed = parse_paste(paste, None);
    assert_eq!(parsed.cookies.len(), 3, "issues: {:?}", codes(&parsed));
    assert_eq!(cookie(&parsed, "a").same_site, 2);
    assert_eq!(cookie(&parsed, "b").same_site, 1);
    assert_eq!(cookie(&parsed, "c").same_site, 0);
    assert_eq!(cookie(&parsed, "c").expires, 0);
    assert!(!has_code(&parsed, "SAME_SITE_UNRECOGNIZED"));
    // Every one of these is Secure, so nothing should warn about None.
    assert!(!has_code(&parsed, "SAME_SITE_NONE_INSECURE"));
  }

  #[test]
  fn test_puppeteer_expires_key_is_read() {
    let paste = r#"[{"name":"sid","value":"v","domain":".example.com","path":"/","expires":2145916800,"httpOnly":true,"secure":true,"sameSite":"Lax"}]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(
      cookie(&parsed, "sid").expires,
      2_145_916_800,
      "reading only expirationDate turned every Puppeteer export into a session cookie"
    );
  }

  #[test]
  fn test_webdriver_expiry_key_is_read() {
    let paste = r#"[{"name":"sid","value":"v","domain":"example.com","path":"/","expiry":2145916800,"secure":false,"httpOnly":false}]"#;
    let parsed = parse_paste(paste, None);
    assert_eq!(cookie(&parsed, "sid").expires, 2_145_916_800);
  }

  #[test]
  fn test_adspower_shaped_payload() {
    // AdsPower's documented shape: string booleans, a numeric-string expiry,
    // and Cookie-Editor's sameSite spelling.
    let paste = r#"[
      {
        "domain": ".example.com",
        "expirationDate": "2145916800",
        "hostOnly": "false",
        "httpOnly": "true",
        "name": "session_id",
        "path": "/",
        "sameSite": "no_restriction",
        "secure": "true",
        "session": "false",
        "value": "abcdef"
      },
      {
        "domain": "example.com",
        "hostOnly": "true",
        "name": "lang",
        "path": "/",
        "sameSite": "unspecified",
        "session": "true",
        "value": "en"
      }
    ]"#;

    let parsed = parse_paste(paste, None);
    assert_eq!(parsed.cookies.len(), 2, "issues: {:?}", codes(&parsed));

    let session = cookie(&parsed, "session_id");
    assert!(session.is_http_only && session.is_secure);
    assert_eq!(session.expires, 2_145_916_800);
    assert_eq!(session.same_site, 0);
    assert_eq!(session.domain, ".example.com");

    let lang = cookie(&parsed, "lang");
    assert_eq!(lang.expires, 0);
    assert_eq!(lang.same_site, -1);
    assert_eq!(lang.domain, "example.com");

    // The string booleans were coerced, and that was reported rather than
    // being silently read as false.
    assert!(has_code(&parsed, "BOOL_COERCED_FROM_STRING"));
  }

  #[test]
  fn test_host_only_reconciliation_both_directions() {
    let paste = r#"[
      {"name":"wide","value":"1","domain":"example.com","hostOnly":false,"path":"/"},
      {"name":"narrow","value":"2","domain":".example.com","hostOnly":true,"path":"/"},
      {"name":"quiet","value":"3","domain":".example.com","hostOnly":false,"path":"/"}
    ]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(cookie(&parsed, "wide").domain, ".example.com");
    assert_eq!(cookie(&parsed, "narrow").domain, "example.com");
    assert_eq!(cookie(&parsed, "quiet").domain, ".example.com");
    assert_eq!(
      parsed
        .issues
        .iter()
        .filter(|i| i.code == "HOST_ONLY_MISMATCH")
        .count(),
      2,
      "only the two contradictions are reported"
    );
  }

  #[test]
  fn test_netscape_include_subdomains_column_reconciles_like_host_only() {
    let paste = format!(
      "{}\n{}\n",
      tsv(&["example.com", "TRUE", "/", "FALSE", "0", "wide", "1"]),
      tsv(&[".example.com", "FALSE", "/", "FALSE", "0", "narrow", "2"]),
    );
    let parsed = parse_paste(&paste, None);

    assert_eq!(cookie(&parsed, "wide").domain, ".example.com");
    assert_eq!(cookie(&parsed, "narrow").domain, "example.com");
  }

  #[test]
  fn test_cookie_request_header() {
    let parsed = parse_paste(
      "Cookie: sid=abc; theme=dark; _ga=GA1.2.3",
      Some("https://example.com/login"),
    );

    assert_eq!(parsed.format, Some(PasteFormat::NameValue));
    assert_eq!(parsed.cookies.len(), 3, "issues: {:?}", codes(&parsed));
    for c in &parsed.cookies {
      assert_eq!(c.domain, "example.com");
      assert_eq!(c.path, "/");
      assert_eq!(c.expires, 0);
      assert_eq!(c.same_site, -1);
      assert!(!c.is_http_only);
      assert!(c.is_secure, "an https site marks the cookies secure");
    }
    assert_eq!(cookie(&parsed, "sid").value, "abc");
  }

  #[test]
  fn test_request_header_pairs_are_never_read_as_attributes() {
    // `priority`, `version` and `comment` are real application cookie names,
    // and a request header has no attributes for them to be mistaken for.
    let parsed = parse_paste(
      "Cookie: sid=1; priority=high; version=3; comment=hi; theme=dark",
      Some("example.com"),
    );

    assert_eq!(parsed.cookies.len(), 5, "issues: {:?}", codes(&parsed));
    assert_eq!(cookie(&parsed, "priority").value, "high");
    assert_eq!(cookie(&parsed, "comment").value, "hi");

    // A colliding name must not leak its value onto the line's other cookies.
    let secure = parse_paste("Cookie: sid=1; secure=yes", Some("example.com"));
    assert_eq!(secure.cookies.len(), 2, "issues: {:?}", codes(&secure));
    assert_eq!(cookie(&secure, "secure").value, "yes");
    assert!(
      !cookie(&secure, "sid").is_secure,
      "a cookie named `secure` must not promote its neighbours"
    );

    let path = parse_paste("Cookie: sid=1; path=/admin", Some("example.com"));
    assert_eq!(cookie(&path, "sid").path, "/");
    assert_eq!(cookie(&path, "path").value, "/admin");
  }

  #[test]
  fn test_set_cookie_attribute_that_swallows_a_value_is_reported() {
    // `Priority`'s value is parsed and then dropped, so consuming a pair on it
    // has to leave a trace; `Path`'s value visibly shapes the cookie, so it
    // does not.
    let parsed = parse_paste("sid=1; Priority=High; Path=/app", Some("example.com"));

    assert_eq!(parsed.cookies.len(), 1);
    assert_eq!(cookie(&parsed, "sid").path, "/app");
    let reported = parsed
      .issues
      .iter()
      .find(|i| i.code == "PAIR_TREATED_AS_ATTRIBUTE")
      .expect("swallowed pair issue");
    assert_eq!(
      reported.params.get("name").map(String::as_str),
      Some("Priority")
    );
    assert_eq!(
      parsed
        .issues
        .iter()
        .filter(|i| i.code == "PAIR_TREATED_AS_ATTRIBUTE")
        .count(),
      1,
      "an attribute whose value is used is not a swallowed pair"
    );

    // Valueless attributes are not pairs and stay quiet.
    let flags = parse_paste("sid=1; Secure; HttpOnly", Some("example.com"));
    assert!(!has_code(&flags, "PAIR_TREATED_AS_ATTRIBUTE"));
  }

  #[test]
  fn test_a_stray_tab_does_not_reroute_a_header_into_netscape() {
    let parsed = parse_paste("Cookie: sid=abc;\ttheme=dark", Some("a.com"));

    assert_eq!(parsed.format, Some(PasteFormat::NameValue));
    assert_eq!(parsed.cookies.len(), 2, "issues: {:?}", codes(&parsed));
    assert!(!has_code(&parsed, "NETSCAPE_FIELD_COUNT"));
  }

  #[test]
  fn test_broken_json_is_diagnosed_as_json() {
    // A trailing comma from a truncated clipboard used to come back as
    // "unrecognized format", or worse, be parsed as name=value pairs.
    let parsed = parse_paste(r#"[{"name":"a","value":"b",}]"#, None);

    assert_eq!(parsed.format, Some(PasteFormat::Json));
    assert!(parsed.cookies.is_empty());
    let reported = parsed
      .issues
      .iter()
      .find(|i| i.code == "JSON_PARSE_FAILED")
      .expect("parse failure issue");
    assert!(
      reported
        .params
        .get("message")
        .is_some_and(|m| m.contains("line")),
      "serde's line and column are what make this actionable"
    );
    assert!(!has_code(&parsed, "UNRECOGNIZED_FORMAT"));

    let misshapen = parse_paste(r#"{"cookies":"nope"}"#, None);
    assert_eq!(misshapen.format, Some(PasteFormat::Json));
    assert!(has_code(&misshapen, "JSON_NOT_COOKIE_LIST"));
  }

  #[test]
  fn test_extreme_expiries_are_bounded_and_never_overflow() {
    let inputs = [
      r#"[{"name":"sid","value":"v","domain":"a.com","expirationDate":-9223372036854775808}]"#
        .to_string(),
      r#"[{"name":"sid","value":"v","domain":"a.com","expirationDate":-1e30}]"#.to_string(),
      r#"[{"name":"sid","value":"v","domain":"a.com","expires":"-9223372036854775808"}]"#
        .to_string(),
      r#"[{"name":"sid","value":"v","domain":"a.com","expirationDate":1e30}]"#.to_string(),
    ];

    for input in &inputs {
      let parsed = parse_paste(input, None);
      let sid = cookie(&parsed, "sid");
      assert!(
        (0..=MAX_PLAUSIBLE_EXPIRY_SECS).contains(&sid.expires),
        "{input} produced {}",
        sid.expires
      );
    }

    // The same magnitudes arriving through a `Set-Cookie` date attribute.
    let header = parse_paste("sid=1; Expires=-9223372036854775808", Some("example.com"));
    assert_eq!(cookie(&header, "sid").expires, 0);
  }

  #[test]
  fn test_issue_params_never_carry_a_cookie_value() {
    let paste = r#"[
      {"name":"sid","value":9223372036854775,"domain":"example.com"},
      {"name":"flag","value":true,"domain":"example.com"}
    ]"#;
    let parsed = parse_paste(paste, None);

    assert!(has_code(&parsed, "VALUE_COERCED"));
    for reported in &parsed.issues {
      for value in reported.params.values() {
        assert!(
          !value.contains("9223372036854775"),
          "a pasted credential must never reach the issue list: {reported:?}"
        );
      }
    }
  }

  #[test]
  fn test_bare_host_site_does_not_invent_a_scheme() {
    let parsed = parse_paste("sid=abc", Some("example.com"));
    assert!(!cookie(&parsed, "sid").is_secure);
    assert_eq!(cookie(&parsed, "sid").domain, "example.com");
  }

  #[test]
  fn test_name_value_without_a_site_asks_for_one() {
    let parsed = parse_paste("sid=abc; theme=dark", None);

    assert_eq!(parsed.format, Some(PasteFormat::NameValue));
    assert!(parsed.site_required);
    assert!(
      parsed.cookies.is_empty(),
      "guessing a domain is not allowed"
    );
    assert!(has_code(&parsed, "SITE_REQUIRED"));
  }

  #[test]
  fn test_set_cookie_line_with_attributes() {
    let parsed = parse_paste(
      "set-cookie: sid=abc123; Path=/app; Domain=.example.com; Expires=Wed, 21 Oct 2015 07:28:00 GMT; HttpOnly; Secure; SameSite=Strict",
      Some("example.com"),
    );

    assert_eq!(parsed.cookies.len(), 1, "issues: {:?}", codes(&parsed));
    let sid = cookie(&parsed, "sid");
    assert_eq!(sid.value, "abc123");
    assert_eq!(sid.path, "/app");
    assert!(sid.is_http_only && sid.is_secure);
    assert_eq!(sid.same_site, 2);
    assert_eq!(sid.expires, 1_445_412_480);
    assert_eq!(parsed.expired_count, 1, "2015 is long gone");
    // The Domain attribute is reported, never used to widen the scope behind
    // the user's back.
    assert!(has_code(&parsed, "DOMAIN_ATTRIBUTE_IGNORED"));
    assert_eq!(sid.domain, "example.com");
  }

  #[test]
  fn test_line_starting_with_domain_is_a_cookie_named_domain() {
    let parsed = parse_paste("domain=eu; sid=1", Some("example.com"));

    assert_eq!(parsed.cookies.len(), 2, "issues: {:?}", codes(&parsed));
    assert_eq!(cookie(&parsed, "domain").value, "eu");
    assert_eq!(cookie(&parsed, "sid").value, "1");
    assert!(!has_code(&parsed, "DOMAIN_ATTRIBUTE_IGNORED"));
  }

  #[test]
  fn test_attribute_after_a_pair_is_an_attribute() {
    let parsed = parse_paste("sid=1; domain=other.test", Some("example.com"));
    assert_eq!(parsed.cookies.len(), 1);
    assert_eq!(cookie(&parsed, "sid").domain, "example.com");
    assert!(has_code(&parsed, "DOMAIN_ATTRIBUTE_IGNORED"));
  }

  #[test]
  fn test_base64_values_keep_their_padding() {
    let parsed = parse_paste(
      "token=YWJjZGVmZ2g=; wide=YWJjZA==; nested=a=b=c",
      Some("example.com"),
    );

    assert_eq!(cookie(&parsed, "token").value, "YWJjZGVmZ2g=");
    assert_eq!(cookie(&parsed, "wide").value, "YWJjZA==");
    assert_eq!(cookie(&parsed, "nested").value, "a=b=c");
  }

  #[test]
  fn test_quoted_value_keeps_quotes_and_says_so() {
    let parsed = parse_paste("sid=\"abc\"", Some("example.com"));
    assert_eq!(cookie(&parsed, "sid").value, "\"abc\"");
    assert!(has_code(&parsed, "QUOTED_VALUE"));
  }

  #[test]
  fn test_max_age_zero_drops_the_cookie() {
    let parsed = parse_paste("sid=deleted; Max-Age=0; Path=/", Some("example.com"));
    assert!(parsed.cookies.is_empty());
    assert!(has_code(&parsed, "MAX_AGE_DELETION"));

    let negative = parse_paste("sid=deleted; Max-Age=-1", Some("example.com"));
    assert!(negative.cookies.is_empty());
    assert!(has_code(&negative, "MAX_AGE_DELETION"));
  }

  #[test]
  fn test_max_age_wins_over_expires() {
    let now = now_secs();
    let parsed = parse_paste(
      "sid=1; Max-Age=3600; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
      Some("example.com"),
    );

    let expires = cookie(&parsed, "sid").expires;
    assert!(
      (expires - (now + 3600)).abs() <= 5,
      "expected roughly now+3600, got {expires}"
    );
    assert_eq!(parsed.expired_count, 0);
  }

  #[test]
  fn test_multiple_lines_cover_multiple_sites_worth_of_pairs() {
    let parsed = parse_paste("a=1; b=2\nc=3", Some("example.com"));
    assert_eq!(parsed.cookies.len(), 3);
    assert_eq!(
      parsed
        .issues
        .iter()
        .filter(|i| i.code == "NAME_VALUE_NO_PAIR")
        .count(),
      0
    );
  }

  #[test]
  fn test_millisecond_expiry_is_converted() {
    let paste =
      r#"[{"name":"sid","value":"v","domain":"example.com","expirationDate":2145916800000}]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(cookie(&parsed, "sid").expires, 2_145_916_800);
    assert!(has_code(&parsed, "EXPIRY_MILLISECONDS"));
  }

  #[test]
  fn test_domain_falls_back_to_the_site_only_when_one_was_given() {
    let paste = r#"[{"name":"sid","value":"v","path":"/"}]"#;

    let with_site = parse_paste(paste, Some("https://example.com"));
    assert_eq!(cookie(&with_site, "sid").domain, "example.com");
    assert!(has_code(&with_site, "DOMAIN_FROM_SITE"));

    let without_site = parse_paste(paste, None);
    assert!(without_site.cookies.is_empty());
    assert!(has_code(&without_site, "DOMAIN_MISSING"));
  }

  #[test]
  fn test_json_name_problems_get_distinct_codes() {
    let paste = r#"[
      {"value":"1","domain":"a.com"},
      {"name":"","value":"2","domain":"a.com"},
      {"name":"bad name","value":"3","domain":"a.com"},
      {"name":42,"value":"4","domain":"a.com"},
      "not an object"
    ]"#;
    let parsed = parse_paste(paste, None);

    assert!(parsed.cookies.is_empty());
    assert!(has_code(&parsed, "NAME_MISSING"));
    assert!(has_code(&parsed, "NAME_EMPTY"));
    assert!(has_code(&parsed, "NAME_INVALID"));
    assert!(has_code(&parsed, "JSON_ENTRY_NOT_OBJECT"));
    let missing = parsed
      .issues
      .iter()
      .find(|i| i.code == "NAME_MISSING")
      .expect("name missing issue");
    assert_eq!(missing.source.as_deref(), Some("cookie 1"));
  }

  #[test]
  fn test_path_is_repaired_and_reported() {
    let paste = r#"[{"name":"sid","value":"v","domain":"a.com","path":"app"}]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(cookie(&parsed, "sid").path, "/app");
    assert!(has_code(&parsed, "PATH_REPAIRED"));
  }

  #[test]
  fn test_unrecognized_same_site_falls_back_to_unspecified() {
    let paste = r#"[{"name":"sid","value":"v","domain":"a.com","sameSite":"sometimes"}]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(
      cookie(&parsed, "sid").same_site,
      -1,
      "an unknown dialect must not become an explicit None"
    );
    assert!(has_code(&parsed, "SAME_SITE_UNRECOGNIZED"));
  }

  #[test]
  fn test_raw_integer_same_site_is_accepted_verbatim() {
    let paste = r#"[
      {"name":"a","value":"1","domain":"a.com","sameSite":2},
      {"name":"b","value":"1","domain":"a.com","sameSite":-1},
      {"name":"c","value":"1","domain":"a.com","sameSite":9}
    ]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(cookie(&parsed, "a").same_site, 2);
    assert_eq!(cookie(&parsed, "b").same_site, -1);
    assert_eq!(cookie(&parsed, "c").same_site, -1);
    assert!(has_code(&parsed, "SAME_SITE_UNRECOGNIZED"));
  }

  #[test]
  fn test_invalid_value_is_rejected() {
    let paste = r#"[{"name":"sid","value":"a;b","domain":"a.com"}]"#;
    let parsed = parse_paste(paste, None);

    assert!(parsed.cookies.is_empty());
    assert!(has_code(&parsed, "VALUE_INVALID"));
  }

  #[test]
  fn test_domain_is_normalised_and_malformed_ones_are_rejected() {
    let paste = r#"[
      {"name":"a","value":"1","domain":"EXAMPLE.COM."},
      {"name":"b","value":"1","domain":"http://example.com/x"},
      {"name":"c","value":"1","domain":"   "}
    ]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(cookie(&parsed, "a").domain, "example.com");
    assert!(parsed.cookies.iter().all(|c| c.name != "b"));
    assert!(has_code(&parsed, "DOMAIN_INVALID"));
    // A blank domain is "absent", so with no site it is the missing-domain case.
    assert!(has_code(&parsed, "DOMAIN_MISSING"));
  }

  #[test]
  fn test_duplicates_keep_the_last_and_report_the_first() {
    let paste = r#"[
      {"name":"sid","value":"old","domain":"a.com","path":"/"},
      {"name":"sid","value":"new","domain":"a.com","path":"/"},
      {"name":"sid","value":"other-path","domain":"a.com","path":"/x"}
    ]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(parsed.cookies.len(), 2);
    assert_eq!(cookie(&parsed, "sid").value, "new");
    let duplicate = parsed
      .issues
      .iter()
      .find(|i| i.code == "DUPLICATE_COOKIE")
      .expect("duplicate issue");
    assert_eq!(duplicate.source.as_deref(), Some("cookie 1"));
  }

  #[test]
  fn test_expired_cookies_are_counted_but_still_returned() {
    let paste = r#"[
      {"name":"old","value":"1","domain":"a.com","expirationDate":1000000000},
      {"name":"live","value":"1","domain":"a.com","expirationDate":2145916800},
      {"name":"session","value":"1","domain":"a.com"}
    ]"#;
    let parsed = parse_paste(paste, None);

    assert_eq!(parsed.cookies.len(), 3);
    assert_eq!(parsed.expired_count, 1);
  }

  #[test]
  fn test_empty_input() {
    for input in ["", "   ", "\n\n", "\u{feff}"] {
      let parsed = parse_paste(input, Some("example.com"));
      assert_eq!(parsed.format, None);
      assert!(parsed.cookies.is_empty());
      assert!(!parsed.site_required);
      assert_eq!(parsed.expired_count, 0);
      assert!(has_code(&parsed, "EMPTY_INPUT"), "input: {input:?}");
    }
  }

  #[test]
  fn test_unrecognized_input_is_reported_never_silently_empty() {
    let parsed = parse_paste("this is just a sentence", None);
    assert_eq!(parsed.format, None);
    assert!(parsed.cookies.is_empty());
    assert!(has_code(&parsed, "UNRECOGNIZED_FORMAT"));
  }

  #[test]
  fn test_malformed_input_of_every_shape_never_panics() {
    let inputs = [
      "[",
      "{",
      "[{}]",
      "[null]",
      "[1,2,3]",
      "{\"cookies\":\"nope\"}",
      "{\"cookies\":[[]]}",
      "\t\t\t\t\t\t",
      "\t",
      "a\tb",
      "#HttpOnly_",
      "#HttpOnly_\t\t\t\t\t\t",
      "# Netscape HTTP Cookie File",
      "=",
      ";;;;",
      "=;=;=",
      "Cookie:",
      "Cookie: ",
      "sid",
      "sid=",
      "=value",
      "\u{0}\u{1}\u{2}",
      "sid=\u{7f}bad",
      "домен=значение",
      &"a".repeat(10_000),
      &format!("sid={}", "x".repeat(10_000)),
      &tsv(&["", "", "", "", "", "", ""]),
      &tsv(&[".a.com", "MAYBE", "x", "MAYBE", "-5", "n", "v"]),
      &tsv(&[
        ".a.com",
        "TRUE",
        "/",
        "TRUE",
        "99999999999999999999",
        "n",
        "v",
      ]),
      "[{\"name\":\"a\",\"domain\":{},\"value\":[]}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"expires\":{}}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"sameSite\":{}}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"secure\":{}}]",
      // Every route an expiry at the ends of the i64 range can take in.
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"expirationDate\":-9223372036854775808}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"expirationDate\":9223372036854775807}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"expirationDate\":-1e30}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"expirationDate\":1e30}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"expires\":\"-9223372036854775808\"}]",
      "[{\"name\":\"a\",\"domain\":\"a.com\",\"expiry\":\"nan\"}]",
      "sid=1; Expires=-9223372036854775808",
      "sid=1; Expires=9223372036854775807",
      "sid=1; Max-Age=9223372036854775807",
      "[{\"name\":\"a\",\"value\":\"b\",}]",
      "{\"cookies\":[",
      "Cookie: sid=1; secure=yes; path=x; expires=nope",
    ];

    for input in inputs {
      for site in [None, Some("example.com"), Some("https://a.b/c"), Some("  ")] {
        let parsed = parse_paste(input, site);
        // Whatever happens, the result stays internally consistent: nothing is
        // reported as parsed without a format, and nothing is silently empty.
        if parsed.cookies.is_empty() && parsed.issues.is_empty() {
          panic!("silent empty result for {input:?} / {site:?}");
        }
        assert!(parsed.expired_count <= parsed.cookies.len());
        if !parsed.cookies.is_empty() {
          assert!(parsed.format.is_some());
        }
        for cookie in &parsed.cookies {
          assert!(!cookie.name.is_empty());
          assert!(!cookie.domain.is_empty());
          assert!(cookie.path.starts_with('/'));
          assert!((-1..=2).contains(&cookie.same_site));
          assert!(cookie.expires >= 0);
          assert!(cookie.expires <= MAX_PLAUSIBLE_EXPIRY_SECS);
        }
      }
      // The detector must agree with itself no matter how odd the input is.
      assert_eq!(detect_format(input), detect_format(input));
    }
  }

  #[test]
  fn test_huge_netscape_expiry_is_clamped_not_overflowed() {
    let paste = tsv(&[
      ".a.com",
      "TRUE",
      "/",
      "TRUE",
      "99999999999999999999",
      "sid",
      "v",
    ]);
    let parsed = parse_paste(&paste, None);

    assert_eq!(cookie(&parsed, "sid").expires, MAX_PLAUSIBLE_EXPIRY_SECS);
    assert!(has_code(&parsed, "EXPIRY_CLAMPED"));
  }

  #[test]
  fn test_single_json_object_is_treated_as_one_cookie() {
    let parsed = parse_paste(
      r#"{"name":"sid","value":"v","domain":"example.com","path":"/","secure":true}"#,
      None,
    );
    assert_eq!(parsed.cookies.len(), 1);
    assert!(cookie(&parsed, "sid").is_secure);
  }
}
