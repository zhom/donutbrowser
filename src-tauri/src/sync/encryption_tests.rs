use super::*;

#[test]
fn test_encrypt_decrypt_roundtrip() {
  let key = [42u8; 32];
  let plaintext = b"Hello, World!";
  let encrypted = encrypt_bytes(&key, plaintext).unwrap();
  let decrypted = decrypt_bytes(&key, &encrypted).unwrap();
  assert_eq!(decrypted, plaintext);
}

#[test]
fn test_encrypt_decrypt_empty_data() {
  let key = [1u8; 32];
  let plaintext = b"";
  let encrypted = encrypt_bytes(&key, plaintext).unwrap();
  let decrypted = decrypt_bytes(&key, &encrypted).unwrap();
  assert_eq!(decrypted, plaintext.to_vec());
}

#[test]
fn test_encrypt_decrypt_large_data() {
  let key = [7u8; 32];
  let plaintext = vec![0xABu8; 1_048_576]; // 1MB
  let encrypted = encrypt_bytes(&key, &plaintext).unwrap();
  let decrypted = decrypt_bytes(&key, &encrypted).unwrap();
  assert_eq!(decrypted, plaintext);
}

#[test]
fn test_different_keys_different_ciphertext() {
  let key1 = [1u8; 32];
  let key2 = [2u8; 32];
  let plaintext = b"same data";
  let encrypted1 = encrypt_bytes(&key1, plaintext).unwrap();
  let encrypted2 = encrypt_bytes(&key2, plaintext).unwrap();
  // Nonces are random so ciphertexts will differ regardless,
  // but decrypting with wrong key should fail
  assert!(decrypt_bytes(&key2, &encrypted1).is_err());
  assert!(decrypt_bytes(&key1, &encrypted2).is_err());
}

#[test]
fn test_nonce_uniqueness() {
  let key = [5u8; 32];
  let plaintext = b"same data encrypted twice";
  let encrypted1 = encrypt_bytes(&key, plaintext).unwrap();
  let encrypted2 = encrypt_bytes(&key, plaintext).unwrap();
  // Different nonces should produce different ciphertext
  assert_ne!(encrypted1, encrypted2);
  // But both should decrypt to the same plaintext
  assert_eq!(
    decrypt_bytes(&key, &encrypted1).unwrap(),
    decrypt_bytes(&key, &encrypted2).unwrap()
  );
}

#[test]
fn test_wrong_key_fails() {
  let key = [10u8; 32];
  let wrong_key = [20u8; 32];
  let plaintext = b"secret data";
  let encrypted = encrypt_bytes(&key, plaintext).unwrap();
  assert!(decrypt_bytes(&wrong_key, &encrypted).is_err());
}

#[test]
fn test_key_derivation_deterministic() {
  let salt = generate_salt();
  let key1 = derive_profile_key("my_password", &salt).unwrap();
  let key2 = derive_profile_key("my_password", &salt).unwrap();
  assert_eq!(key1, key2);
}

#[test]
fn test_key_derivation_different_salts() {
  let salt1 = generate_salt();
  let salt2 = generate_salt();
  let key1 = derive_profile_key("my_password", &salt1).unwrap();
  let key2 = derive_profile_key("my_password", &salt2).unwrap();
  assert_ne!(key1, key2);
}

#[test]
fn test_salt_generation_unique() {
  let salt1 = generate_salt();
  let salt2 = generate_salt();
  assert_ne!(salt1, salt2);
}

#[test]
fn test_password_storage_roundtrip() {
  let password = "test_password_12345";
  store_e2e_password(password).unwrap();
  assert!(has_e2e_password());
  let loaded = load_e2e_password().unwrap();
  assert_eq!(loaded, Some(password.to_string()));
  remove_e2e_password().unwrap();
  assert!(!has_e2e_password());
}

#[test]
fn test_decrypt_too_short_data() {
  let key = [1u8; 32];
  assert!(decrypt_bytes(&key, &[0u8; 5]).is_err());
}
