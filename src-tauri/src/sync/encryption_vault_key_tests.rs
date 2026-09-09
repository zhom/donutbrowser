use super::{decode_salt, derive_vault_key, encode_salt};

/// A stored vault is only readable while this vector holds. It pins the
/// Argon2id parameters and the salt encoding together: a dependency bump
/// that changed either would fail here instead of at the user's data.
#[test]
fn vault_key_derivation_is_pinned() {
  let key = derive_vault_key(b"correct horse battery staple", &[7u8; 16]).unwrap();
  let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
  assert_eq!(
    hex,
    "799f12b9e17710824482d829835acb69f5a9355bf774c4f07342823b11b90928"
  );
}

#[test]
fn salt_encoding_round_trips_without_padding() {
  let salt = [0u8, 1, 2, 3, 250, 251, 252, 253, 254, 255, 9, 8, 7, 6, 5, 4];
  let encoded = encode_salt(&salt);
  assert!(!encoded.contains('='), "PHC B64 carries no padding");
  assert_eq!(decode_salt(&encoded).unwrap(), salt);
  assert!(decode_salt("not*valid").is_err());
}
