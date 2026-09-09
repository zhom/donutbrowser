//! The person a profile presents as, when a site asks for one.
//!
//! Wayfern shows a "Fill with generated" submenu in any text field, built
//! from a document the launcher writes: `{"fields":[{"label","value"},…]}`.
//! The browser never invents a value, so everything here is donut's.
//!
//! A persona is DERIVED, not stored as prose: the same profile hands the
//! browser the same person on every launch, and two profiles never share one,
//! because every field is a function of the profile's own seed. The user can
//! still edit any field; edits are the only thing that persists.

use serde::{Deserialize, Serialize};

/// A named value the browser offers in its fill submenu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaField {
  pub id: String,
  pub label: String,
  pub value: String,
}

/// The browser truncates a longer submenu; keeping the same bound here means
/// what the user edits is what the browser shows.
const MAX_FIELDS: usize = 24;
/// Bounds that match what the fill submenu will render, so a value is never
/// silently shortened.
const MAX_LABEL_CHARS: usize = 64;
const MAX_VALUE_CHARS: usize = 512;

/// The fields a derived persona carries, in submenu order.
pub const FIELD_IDS: [&str; 9] = [
  "full_name",
  "first_name",
  "last_name",
  "email",
  "username",
  "phone",
  "birth_date",
  "street_address",
  "postal_code",
];

/// FNV-1a with the stream number folded into the initial state, then
/// splitmix64, so neighbouring streams do not produce visibly related values.
fn draw(seed: &str, stream: u64) -> u64 {
  let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ stream;
  for byte in seed.as_bytes() {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
  }
  let mut z = hash.wrapping_add(0x9e37_79b9_7f4a_7c15);
  z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
  z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
  z ^ (z >> 31)
}

fn pick<'a>(seed: &str, stream: u64, options: &[&'a str]) -> &'a str {
  options[(draw(seed, stream) % options.len() as u64) as usize]
}

const GIVEN_NAMES: [&str; 32] = [
  "Amelia", "Noah", "Sofia", "Liam", "Mia", "Lucas", "Emma", "Ethan", "Olivia", "Mateo", "Ava",
  "Leon", "Zara", "Hugo", "Nora", "Adam", "Iris", "Felix", "Maya", "Oscar", "Lena", "Rafael",
  "Clara", "Milan", "Elif", "Jonas", "Nina", "Tobias", "Rosa", "Kai", "Alma", "Viktor",
];

const FAMILY_NAMES: [&str; 32] = [
  "Bennett",
  "Novak",
  "Marchetti",
  "Okafor",
  "Lindqvist",
  "Haddad",
  "Vasquez",
  "Ferreira",
  "Kowalski",
  "Dubois",
  "Andersen",
  "Rahman",
  "Moretti",
  "Kaminski",
  "Bauer",
  "Silva",
  "Petrov",
  "Nakamura",
  "Kelly",
  "Weiss",
  "Salgado",
  "Virtanen",
  "Costa",
  "Yilmaz",
  "Horvat",
  "Laurent",
  "Fischer",
  "Blake",
  "Reyes",
  "Janssen",
  "Meyer",
  "Sorensen",
];

const STREETS: [&str; 16] = [
  "Maple Avenue",
  "Linden Street",
  "Harbour Road",
  "Kestrel Lane",
  "Alder Way",
  "Foundry Street",
  "Willow Crescent",
  "Bridgeway",
  "Chandler Street",
  "Orchard Row",
  "Beacon Hill",
  "Cypress Walk",
  "Quarry Road",
  "Sable Street",
  "Juniper Court",
  "Pier Lane",
];

const MAIL_HOSTS: [&str; 6] = [
  "gmail.com",
  "outlook.com",
  "proton.me",
  "yahoo.com",
  "icloud.com",
  "fastmail.com",
];

/// A calendar date `years_back` years or so before now, as `YYYY-MM-DD`.
/// Days-in-month is handled by capping at 28, which every month has.
fn birth_date(seed: &str) -> String {
  let year = 1970 + (draw(seed, 61) % 36); // 1970..2005: adult in any locale
  let month = 1 + (draw(seed, 62) % 12);
  let day = 1 + (draw(seed, 63) % 28);
  format!("{year:04}-{month:02}-{day:02}")
}

/// Digits only, so the value is usable in a field with any formatting rule.
fn phone(seed: &str) -> String {
  let area = 200 + (draw(seed, 71) % 700);
  let prefix = 200 + (draw(seed, 72) % 700);
  let line = draw(seed, 73) % 10_000;
  format!("+1{area:03}{prefix:03}{line:04}")
}

/// Derive the persona a profile presents, in submenu order.
///
/// `seed` must be stable for the profile and unique to it: the identity id
/// when it has one, otherwise the profile id. Nothing here reads the clock or
/// the host, so the same seed reproduces the same person anywhere.
pub fn derive(seed: &str) -> Vec<PersonaField> {
  let given = pick(seed, 11, &GIVEN_NAMES);
  let family = pick(seed, 12, &FAMILY_NAMES);
  let username = format!(
    "{}{}{}",
    given.to_lowercase(),
    family.to_lowercase(),
    draw(seed, 21) % 100
  );
  let email = format!("{username}@{}", pick(seed, 22, &MAIL_HOSTS));
  let street = format!(
    "{} {}",
    1 + (draw(seed, 31) % 200),
    pick(seed, 32, &STREETS)
  );
  let postal = format!("{:05}", draw(seed, 33) % 100_000);

  // Ordered by FIELD_IDS, which is what the browser's submenu shows.
  let labels = [
    "Full name",
    "First name",
    "Last name",
    "Email",
    "Username",
    "Phone",
    "Date of birth",
    "Street address",
    "Postal code",
  ];
  let values = [
    format!("{given} {family}"),
    given.to_string(),
    family.to_string(),
    email,
    username,
    phone(seed),
    birth_date(seed),
    street,
    postal,
  ];
  FIELD_IDS
    .iter()
    .zip(labels)
    .zip(values)
    .map(|((id, label), value)| field(id, label, value))
    .collect()
}

fn field(id: &str, label: &str, value: String) -> PersonaField {
  PersonaField {
    id: id.to_string(),
    label: label.to_string(),
    value,
  }
}

/// Apply the user's edits to a derived persona: an edit replaces the value of
/// the field it names, an unknown id is appended, and a blank value removes
/// the row so the browser never offers an empty entry.
pub fn with_edits(seed: &str, edits: &[PersonaField]) -> Vec<PersonaField> {
  let mut fields = derive(seed);
  for edit in edits {
    let value = edit.value.trim();
    match fields.iter().position(|f| f.id == edit.id) {
      Some(index) if value.is_empty() => {
        fields.remove(index);
      }
      Some(index) => {
        fields[index].value = value.to_string();
        if !edit.label.trim().is_empty() {
          fields[index].label = edit.label.trim().to_string();
        }
      }
      None if value.is_empty() => {}
      None => fields.push(field(
        &edit.id,
        if edit.label.trim().is_empty() {
          &edit.id
        } else {
          edit.label.trim()
        },
        value.to_string(),
      )),
    }
  }
  fields.truncate(MAX_FIELDS);
  for field in &mut fields {
    truncate_chars(&mut field.label, MAX_LABEL_CHARS);
    truncate_chars(&mut field.value, MAX_VALUE_CHARS);
  }
  fields
}

fn truncate_chars(text: &mut String, limit: usize) {
  if text.chars().count() > limit {
    *text = text.chars().take(limit).collect();
  }
}

/// The document the browser reads, as it writes it to disk.
pub fn document(fields: &[PersonaField]) -> serde_json::Value {
  serde_json::json!({ "fields": fields })
}

/// The person this profile presents, as the browser will offer it: derived
/// from the profile's own seed with the user's edits applied.
///
/// The seed is the identity id when the profile has one and its own id
/// otherwise, which is exactly what the launcher uses, so what this returns is
/// what the next launch writes. `derived_only` asks for the person before any
/// edit, which is what "reset to generated" shows.
#[tauri::command]
pub fn get_profile_persona(
  profile_id: String,
  derived_only: Option<bool>,
) -> Result<Vec<PersonaField>, String> {
  let profile = crate::profile::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?
    .into_iter()
    .find(|profile| profile.id.to_string() == profile_id)
    .ok_or_else(|| crate::backend_error("PROFILE_NOT_FOUND"))?;
  let config = profile.wayfern_config.unwrap_or_default();
  let seed = config
    .identity_id
    .as_deref()
    .map(str::trim)
    .filter(|id| !id.is_empty())
    .map(str::to_string)
    .unwrap_or_else(|| profile.id.to_string());
  if derived_only.unwrap_or(false) {
    return Ok(derive(&seed));
  }
  let edits: Vec<PersonaField> = config
    .persona
    .as_deref()
    .map(str::trim)
    .filter(|edits| !edits.is_empty())
    .and_then(|edits| serde_json::from_str(edits).ok())
    .unwrap_or_default();
  Ok(with_edits(&seed, &edits))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_persona_is_stable_for_one_seed_and_different_across_seeds() {
    let a = derive("3fa85f64-5717-4562-b3fc-2c963f66afa6");
    assert_eq!(a, derive("3fa85f64-5717-4562-b3fc-2c963f66afa6"));
    let b = derive("9c858901-8a57-4791-81fe-4c455b099bc9");
    assert_ne!(a, b);
    assert_eq!(
      a.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
      FIELD_IDS,
      "the submenu order is the launcher's decision and must not drift"
    );
  }

  #[test]
  fn every_derived_value_is_usable() {
    for seed in ["a", "b", "seed-3", "3fa85f64-5717-4562-b3fc-2c963f66afa6"] {
      let fields = derive(seed);
      let get = |id: &str| {
        fields
          .iter()
          .find(|f| f.id == id)
          .map(|f| f.value.clone())
          .unwrap()
      };
      assert!(get("email").contains('@'));
      assert!(get("email").starts_with(&get("username")));
      assert!(
        get("full_name") == format!("{} {}", get("first_name"), get("last_name")),
        "the full name must be the two parts it is made of"
      );
      let phone = get("phone");
      assert!(phone.starts_with('+') && phone[1..].chars().all(|c| c.is_ascii_digit()));
      let birth = get("birth_date");
      assert_eq!(birth.len(), 10);
      let day: u32 = birth[8..].parse().unwrap();
      assert!((1..=28).contains(&day), "{birth}");
      assert!(fields.iter().all(|f| !f.value.trim().is_empty()));
    }
  }

  #[test]
  fn an_edit_replaces_one_field_and_a_blank_removes_it() {
    let seed = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let edited = with_edits(
      seed,
      &[
        field("email", "", "me@example.com".into()),
        field("phone", "", "  ".into()),
        field("company", "Company", "Donut".into()),
      ],
    );
    assert_eq!(
      edited.iter().find(|f| f.id == "email").unwrap().value,
      "me@example.com"
    );
    assert!(edited.iter().all(|f| f.id != "phone"));
    let extra = edited.iter().find(|f| f.id == "company").unwrap();
    assert_eq!(
      (extra.label.as_str(), extra.value.as_str()),
      ("Company", "Donut")
    );
    // Everything not edited still comes from the seed.
    let derived = derive(seed);
    assert_eq!(
      edited.iter().find(|f| f.id == "full_name").unwrap().value,
      derived.iter().find(|f| f.id == "full_name").unwrap().value
    );
  }

  #[test]
  fn edits_cannot_exceed_the_browsers_own_limits() {
    let long = "x".repeat(1000);
    let edited = with_edits(
      "seed",
      &(0..40)
        .map(|i| field(&format!("extra{i}"), &long, long.clone()))
        .collect::<Vec<_>>(),
    );
    assert_eq!(edited.len(), MAX_FIELDS);
    assert!(edited
      .iter()
      .all(|f| f.label.chars().count() <= MAX_LABEL_CHARS
        && f.value.chars().count() <= MAX_VALUE_CHARS));
  }

  #[test]
  fn the_document_is_the_shape_the_browser_parses() {
    let document = document(&derive("seed"));
    let fields = document["fields"].as_array().unwrap();
    assert_eq!(fields.len(), FIELD_IDS.len());
    assert!(fields
      .iter()
      .all(|f| f["id"].is_string() && f["label"].is_string() && f["value"].is_string()));
  }
}
