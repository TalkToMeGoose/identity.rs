// Copyright 2020-2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use core::fmt::Debug;
use core::fmt::Formatter;

use async_trait::async_trait;
#[cfg(feature = "encryption")]
use crypto::ciphers::aes_gcm::Aes256Gcm;
#[cfg(feature = "encryption")]
use crypto::ciphers::aes_kw::Aes256Kw;
#[cfg(feature = "encryption")]
use crypto::ciphers::traits::Aead;
use hashbrown::HashMap;
use identity_core::crypto::Ed25519;
use identity_core::crypto::KeyPair;
use identity_core::crypto::KeyType;
use identity_core::crypto::PrivateKey;
use identity_core::crypto::PublicKey;
use identity_core::crypto::Sign;
#[cfg(feature = "encryption")]
use identity_core::crypto::X25519;
use identity_did::did::CoreDID;
use identity_iota_core::did::IotaDID;
use identity_iota_core::tangle::NetworkName;
use std::sync::RwLockReadGuard;
use std::sync::RwLockWriteGuard;
use zeroize::Zeroize;

use crate::error::Error;
use crate::error::Result;
use crate::storage::Storage;
#[cfg(feature = "encryption")]
use crate::types::CekAlgorithm;
use crate::types::DIDType;
#[cfg(feature = "encryption")]
use crate::types::EncryptedData;
#[cfg(feature = "encryption")]
use crate::types::EncryptionAlgorithm;
use crate::types::KeyLocation;
use crate::types::Signature;
use crate::utils::Shared;

// The map from DIDs to vaults.
type Vaults = HashMap<CoreDID, MemVault>;
// The map from key locations to key pairs, that lives within a DID partition.
type MemVault = HashMap<KeyLocation, KeyPair>;

/// An insecure, in-memory [`Storage`] implementation that serves as an example and is used in tests.
pub struct MemStore {
  // Controls whether to print the storages content when debugging.
  expand: bool,
  blobs: Shared<HashMap<CoreDID, Vec<u8>>>,
  vaults: Shared<Vaults>,
}

impl MemStore {
  /// Creates a new, empty `MemStore` instance.
  pub fn new() -> Self {
    Self {
      expand: false,
      blobs: Shared::new(HashMap::new()),
      vaults: Shared::new(HashMap::new()),
    }
  }

  /// Returns whether to expand the debug representation.
  pub fn expand(&self) -> bool {
    self.expand
  }

  /// Sets whether to expand the debug representation.
  pub fn set_expand(&mut self, value: bool) {
    self.expand = value;
  }
}

// Refer to the `Storage` interface docs for high-level documentation of the individual methods.
#[cfg_attr(not(feature = "send-sync-storage"), async_trait(?Send))]
#[cfg_attr(feature = "send-sync-storage", async_trait)]
impl Storage for MemStore {
  async fn did_create(
    &self,
    did_type: DIDType,
    network: NetworkName,
    fragment: &str,
    private_key: Option<PrivateKey>,
  ) -> Result<(CoreDID, KeyLocation)> {
    // Extract a `KeyPair` from the passed private key or generate a new one.
    // For `did_create` we can assume the `KeyType` to be `Ed25519` because
    // that is the only currently available signature type.
    let keypair: KeyPair = match private_key {
      Some(private_key) => KeyPair::try_from_private_key_bytes(KeyType::Ed25519, private_key.as_ref())?,
      None => KeyPair::new(KeyType::Ed25519)?,
    };

    // We create the location at which the key pair will be stored.
    // Most notably, this uses the public key as an input.
    let location: KeyLocation = KeyLocation::new(KeyType::Ed25519, fragment.to_owned(), keypair.public().as_ref());

    // Next we use the public key to derive the initial DID.
    let did: CoreDID = {
      match did_type {
        DIDType::IotaDID => IotaDID::new_with_network(keypair.public().as_ref(), network)
          .map_err(|err| crate::Error::DIDCreationError(err.to_string()))?
          .into(),
      }
    };

    // Obtain exclusive access to the vaults.
    let mut vaults: RwLockWriteGuard<'_, _> = self.vaults.write()?;

    // We use the vaults as the index of DIDs stored in this storage instance.
    // If the DID already exists, we need to return an error. We don't want to overwrite an existing DID.
    if vaults.contains_key(&did) {
      return Err(Error::IdentityAlreadyExists);
    }

    // Obtain the exiting mem vault or create a new one.
    let vault: &mut MemVault = vaults.entry(did.clone()).or_default();

    // Insert the key pair at the previously created location.
    vault.insert(location.clone(), keypair);

    // Return did and location.
    Ok((did, location))
  }

  async fn did_purge(&self, did: &CoreDID) -> Result<bool> {
    // This method is supposed to be idempotent,
    // so we only need to do work if the DID still exists.
    // The return value signals whether the DID was actually removed during this operation.
    if self.vaults.write()?.remove(did).is_some() {
      let _ = self.blobs.write()?.remove(did);
      Ok(true)
    } else {
      Ok(false)
    }
  }

  async fn did_exists(&self, did: &CoreDID) -> Result<bool> {
    // Note that any failure to get access to the storage and do the actual existence check
    // should result in an error rather than returning `false`.
    Ok(self.vaults.read()?.contains_key(did))
  }

  async fn did_list(&self) -> Result<Vec<CoreDID>> {
    Ok(self.vaults.read()?.keys().cloned().collect())
  }

  async fn key_generate(&self, did: &CoreDID, key_type: KeyType, fragment: &str) -> Result<KeyLocation> {
    // Obtain exclusive access to the vaults.
    let mut vaults: RwLockWriteGuard<'_, _> = self.vaults.write()?;
    // Get or insert the MemVault.
    let vault: &mut MemVault = vaults.entry(did.clone()).or_default();

    // Generate a new key pair for the given `key_type`.
    let keypair: KeyPair = KeyPair::new(key_type)?;

    // Derive the key location from the fragment and public key and set the `KeyType` of the location.
    let location: KeyLocation = KeyLocation::new(key_type, fragment.to_owned(), keypair.public().as_ref());

    vault.insert(location.clone(), keypair);

    // Return the location at which the key was generated.
    Ok(location)
  }

  async fn key_insert(&self, did: &CoreDID, location: &KeyLocation, mut private_key: PrivateKey) -> Result<()> {
    // Obtain exclusive access to the vaults.
    let mut vaults: RwLockWriteGuard<'_, _> = self.vaults.write()?;
    // Get or insert the MemVault.
    let vault: &mut MemVault = vaults.entry(did.clone()).or_default();

    // Reconstruct the key pair from the given private key by inspecting the location for its key type.
    // Then insert the key at the given location.
    match location.key_type {
      KeyType::Ed25519 => {
        let keypair: KeyPair = KeyPair::try_from_private_key_bytes(KeyType::Ed25519, private_key.as_ref())
          .map_err(|err| Error::InvalidPrivateKey(err.to_string()))?;
        private_key.zeroize();

        vault.insert(location.to_owned(), keypair);

        Ok(())
      }
      KeyType::X25519 => {
        let keypair: KeyPair = KeyPair::try_from_private_key_bytes(KeyType::X25519, private_key.as_ref())
          .map_err(|err| Error::InvalidPrivateKey(err.to_string()))?;
        private_key.zeroize();

        vault.insert(location.to_owned(), keypair);

        Ok(())
      }
    }
  }

  async fn key_exists(&self, did: &CoreDID, location: &KeyLocation) -> Result<bool> {
    // Obtain read access to the vaults.
    let vaults: RwLockReadGuard<'_, _> = self.vaults.read()?;

    // Within the DID vault, check for existence of the given location.
    if let Some(vault) = vaults.get(did) {
      return Ok(vault.contains_key(location));
    }

    Ok(false)
  }

  async fn key_public(&self, did: &CoreDID, location: &KeyLocation) -> Result<PublicKey> {
    // Obtain read access to the vaults.
    let vaults: RwLockReadGuard<'_, _> = self.vaults.read()?;
    // Lookup the vault for the given DID.
    let vault: &MemVault = vaults.get(did).ok_or(Error::KeyVaultNotFound)?;
    // Lookup the key pair within the vault.
    let keypair: &KeyPair = vault.get(location).ok_or(Error::KeyNotFound)?;

    // Return the public key.
    Ok(keypair.public().clone())
  }

  async fn key_delete(&self, did: &CoreDID, location: &KeyLocation) -> Result<bool> {
    // Obtain read access to the vaults.
    let mut vaults: RwLockWriteGuard<'_, _> = self.vaults.write()?;
    // Lookup the vault for the given DID.
    let vault: &mut MemVault = vaults.get_mut(did).ok_or(Error::KeyVaultNotFound)?;

    // This method is supposed to be idempotent, so we delete the key
    // if it exists and return whether it was actually deleted during this operation.
    Ok(vault.remove(location).is_some())
  }

  async fn key_sign(&self, did: &CoreDID, location: &KeyLocation, data: Vec<u8>) -> Result<Signature> {
    // Obtain read access to the vaults.
    let vaults: RwLockReadGuard<'_, _> = self.vaults.read()?;
    // Lookup the vault for the given DID.
    let vault: &MemVault = vaults.get(did).ok_or(Error::KeyVaultNotFound)?;
    // Lookup the key pair within the vault.
    let keypair: &KeyPair = vault.get(location).ok_or(Error::KeyNotFound)?;

    match location.key_type {
      KeyType::Ed25519 => {
        assert_eq!(keypair.type_(), KeyType::Ed25519);

        // Use the `Ed25519` API to sign the given data with the private key.
        let signature: [u8; 64] = Ed25519::sign(&data, keypair.private())?;
        // Construct a new `Signature` wrapper with the returned signature bytes.
        let signature: Signature = Signature::new(signature.to_vec());
        Ok(signature)
      }
      KeyType::X25519 => {
        // Calling key_sign on key types that cannot be signed with should return an error.
        return Err(identity_did::Error::InvalidMethodType.into());
      }
    }
  }

  #[cfg(feature = "encryption")]
  async fn data_encrypt(
    &self,
    _did: &CoreDID,
    plaintext: Vec<u8>,
    associated_data: Vec<u8>,
    encryption_algorithm: &EncryptionAlgorithm,
    cek_algorithm: &CekAlgorithm,
    public_key: PublicKey,
  ) -> Result<EncryptedData> {
    let public_key: [u8; X25519::PUBLIC_KEY_LENGTH] = public_key
      .as_ref()
      .try_into()
      .map_err(|_| Error::InvalidPublicKey(format!("expected public key of length {}", X25519::PUBLIC_KEY_LENGTH)))?;
    match cek_algorithm {
      CekAlgorithm::ECDH_ES(agreement) => {
        // Generate ephemeral key
        let keypair: KeyPair = KeyPair::new(KeyType::X25519)?;
        // Obtain the shared secret by combining the ephemeral key and the static public key
        let shared_secret: [u8; 32] = X25519::key_exchange(keypair.private(), &public_key)?;
        let derived_secret: Vec<u8> =
          memstore_encryption::concat_kdf(cek_algorithm.name(), Aes256Gcm::KEY_LENGTH, &shared_secret, agreement)
            .map_err(Error::EncryptionFailure)?;
        let encrypted_data = memstore_encryption::try_encrypt(
          &derived_secret,
          encryption_algorithm,
          &plaintext,
          associated_data,
          Vec::new(),
          keypair.public().as_ref().to_vec(),
        )?;
        Ok(encrypted_data)
      }
      CekAlgorithm::ECDH_ES_A256KW(agreement) => {
        let keypair: KeyPair = KeyPair::new(KeyType::X25519)?;
        let shared_secret: [u8; 32] = X25519::key_exchange(keypair.private(), &public_key)?;
        let derived_secret: Vec<u8> =
          memstore_encryption::concat_kdf(cek_algorithm.name(), Aes256Kw::KEY_LENGTH, &shared_secret, agreement)
            .map_err(Error::EncryptionFailure)?;

        let cek: Vec<u8> = memstore_encryption::generate_content_encryption_key(*encryption_algorithm)?;

        let mut encrypted_cek: Vec<u8> = vec![0; cek.len() + Aes256Kw::BLOCK];
        let aes_kw: Aes256Kw<'_> = Aes256Kw::new(derived_secret.as_ref());
        aes_kw
          .wrap_key(cek.as_ref(), &mut encrypted_cek)
          .map_err(Error::EncryptionFailure)?;

        let encrypted_data = memstore_encryption::try_encrypt(
          &cek,
          encryption_algorithm,
          &plaintext,
          associated_data,
          encrypted_cek,
          keypair.public().as_ref().to_vec(),
        )?;
        Ok(encrypted_data)
      }
    }
  }

  #[cfg(feature = "encryption")]
  async fn data_decrypt(
    &self,
    did: &CoreDID,
    data: EncryptedData,
    encryption_algorithm: &EncryptionAlgorithm,
    cek_algorithm: &CekAlgorithm,
    private_key: &KeyLocation,
  ) -> Result<Vec<u8>> {
    // Retrieves the PrivateKey from the vault
    let vaults: RwLockReadGuard<'_, _> = self.vaults.read()?;
    let vault: &MemVault = vaults.get(did).ok_or(Error::KeyVaultNotFound)?;
    let key_pair: &KeyPair = vault.get(private_key).ok_or(Error::KeyNotFound)?;
    // Decrypts the data
    match key_pair.type_() {
      KeyType::Ed25519 => Err(Error::InvalidPrivateKey(
        "Ed25519 keys are not supported for decryption".to_owned(),
      )),
      KeyType::X25519 => {
        let public_key: [u8; X25519::PUBLIC_KEY_LENGTH] =
          data.ephemeral_public_key.clone().try_into().map_err(|_| {
            Error::InvalidPublicKey(format!("expected public key of length {}", X25519::PUBLIC_KEY_LENGTH))
          })?;
        match cek_algorithm {
          CekAlgorithm::ECDH_ES(agreement) => {
            let shared_secret: [u8; 32] = X25519::key_exchange(key_pair.private(), &public_key)?;
            let derived_secret: Vec<u8> =
              memstore_encryption::concat_kdf(cek_algorithm.name(), Aes256Gcm::KEY_LENGTH, &shared_secret, agreement)
                .map_err(Error::DecryptionFailure)?;
            memstore_encryption::try_decrypt(&derived_secret, encryption_algorithm, &data)
          }
          CekAlgorithm::ECDH_ES_A256KW(agreement) => {
            let shared_secret: [u8; 32] = X25519::key_exchange(key_pair.private(), &public_key)?;
            let derived_secret: Vec<u8> =
              memstore_encryption::concat_kdf(cek_algorithm.name(), Aes256Kw::KEY_LENGTH, &shared_secret, agreement)
                .map_err(Error::DecryptionFailure)?;

            let cek_len: usize =
              data
                .encrypted_cek
                .len()
                .checked_sub(Aes256Kw::BLOCK)
                .ok_or(Error::DecryptionFailure(crypto::Error::BufferSize {
                  name: "plaintext cek",
                  needs: Aes256Kw::BLOCK,
                  has: data.encrypted_cek.len(),
                }))?;

            let mut cek: Vec<u8> = vec![0; cek_len];
            let aes_kw: Aes256Kw<'_> = Aes256Kw::new(derived_secret.as_ref());
            aes_kw
              .unwrap_key(data.encrypted_cek.as_ref(), &mut cek)
              .map_err(Error::DecryptionFailure)?;

            memstore_encryption::try_decrypt(&cek, encryption_algorithm, &data)
          }
        }
      }
    }
  }

  async fn blob_set(&self, did: &CoreDID, value: Vec<u8>) -> Result<()> {
    // Set the arbitrary value for the given DID.
    self.blobs.write()?.insert(did.clone(), value);

    Ok(())
  }

  async fn blob_get(&self, did: &CoreDID) -> Result<Option<Vec<u8>>> {
    // Lookup the value stored of the given DID.
    self.blobs.read().map(|data| data.get(did).cloned())
  }

  async fn flush_changes(&self) -> Result<()> {
    // The MemStore doesn't need to flush changes to disk or any other persistent store,
    // which is why this function does nothing.
    Ok(())
  }
}

#[cfg(feature = "encryption")]
mod memstore_encryption {
  use crate::types::AgreementInfo;
  use crate::types::EncryptedData;
  use crate::types::EncryptionAlgorithm;
  use crate::Error;
  use crate::Result;
  use crypto::ciphers::aes_gcm::Aes256Gcm;
  use crypto::ciphers::traits::Aead;
  use crypto::hashes::sha::Sha256;
  use crypto::hashes::Digest;

  pub(crate) fn try_encrypt(
    key: &[u8],
    algorithm: &EncryptionAlgorithm,
    data: &[u8],
    associated_data: Vec<u8>,
    encrypted_cek: Vec<u8>,
    ephemeral_public_key: Vec<u8>,
  ) -> Result<EncryptedData> {
    match algorithm {
      EncryptionAlgorithm::AES256GCM => {
        let nonce: &[u8] = &Aes256Gcm::random_nonce().map_err(Error::EncryptionFailure)?;
        let padding: usize = Aes256Gcm::padsize(data).map(|size| size.get()).unwrap_or_default();
        let mut ciphertext: Vec<u8> = vec![0; data.len() + padding];
        let mut tag: Vec<u8> = [0; Aes256Gcm::TAG_LENGTH].to_vec();
        Aes256Gcm::try_encrypt(key, nonce, associated_data.as_ref(), data, &mut ciphertext, &mut tag)
          .map_err(Error::EncryptionFailure)?;
        Ok(EncryptedData::new(
          nonce.to_vec(),
          associated_data,
          tag,
          ciphertext,
          encrypted_cek,
          ephemeral_public_key,
        ))
      }
    }
  }

  pub(crate) fn try_decrypt(key: &[u8], algorithm: &EncryptionAlgorithm, data: &EncryptedData) -> Result<Vec<u8>> {
    match algorithm {
      EncryptionAlgorithm::AES256GCM => {
        let mut plaintext = vec![0; data.ciphertext.len()];
        let len: usize = Aes256Gcm::try_decrypt(
          key,
          &data.nonce,
          &data.associated_data,
          &mut plaintext,
          &data.ciphertext,
          &data.tag,
        )
        .map_err(Error::DecryptionFailure)?;
        plaintext.truncate(len);
        Ok(plaintext)
      }
    }
  }

  /// The Concat KDF (using SHA-256) as defined in Section 5.8.1 of NIST.800-56A
  pub(crate) fn concat_kdf(
    alg: &'static str,
    len: usize,
    shared_secret: &[u8],
    agreement: &AgreementInfo,
  ) -> crypto::error::Result<Vec<u8>> {
    let mut digest: Sha256 = Sha256::new();
    let mut output: Vec<u8> = Vec::new();

    let target: usize = (len + (Sha256::output_size() - 1)) / Sha256::output_size();
    let rounds: u32 = u32::try_from(target).map_err(|_| crypto::error::Error::InvalidArgumentError {
      alg,
      expected: "iterations can't exceed 2^32 - 1",
    })?;

    for count in 0..rounds {
      // Iteration Count
      digest.update(&(count as u32 + 1).to_be_bytes());

      // Derived Secret
      digest.update(shared_secret);

      // AlgorithmId
      digest.update(&(alg.len() as u32).to_be_bytes());
      digest.update(alg.as_bytes());

      // PartyUInfo
      digest.update(&(agreement.apu.len() as u32).to_be_bytes());
      digest.update(&agreement.apu);

      // PartyVInfo
      digest.update(&(agreement.apv.len() as u32).to_be_bytes());
      digest.update(&agreement.apv);

      // SuppPubInfo
      digest.update(&agreement.pub_info);

      // SuppPrivInfo
      digest.update(&agreement.priv_info);

      output.extend_from_slice(&digest.finalize_reset());
    }

    output.truncate(len);

    Ok(output)
  }

  /// Generate a random content encryption key of suitable length for `encryption_algorithm`.
  pub(crate) fn generate_content_encryption_key(encryption_algorithm: EncryptionAlgorithm) -> Result<Vec<u8>> {
    let mut bytes: Vec<u8> = vec![0; encryption_algorithm.key_length()];
    crypto::utils::rand::fill(bytes.as_mut()).map_err(Error::EncryptionFailure)?;
    Ok(bytes)
  }
}

impl Debug for MemStore {
  fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
    if self.expand {
      f.debug_struct("MemStore")
        .field("blobs", &self.blobs)
        .field("vaults", &self.vaults)
        .finish()
    } else {
      f.write_str("MemStore")
    }
  }
}

impl Default for MemStore {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
#[cfg(feature = "storage-test-suite")]
mod tests {
  use crate::storage::Storage;
  use crate::storage::StorageTestSuite;

  use super::MemStore;

  fn test_memstore() -> impl Storage {
    MemStore::new()
  }

  #[tokio::test]
  async fn test_memstore_did_create_with_private_key() {
    StorageTestSuite::did_create_private_key_test(test_memstore())
      .await
      .unwrap()
  }

  #[tokio::test]
  async fn test_memstore_did_create_generate_key() {
    StorageTestSuite::did_create_generate_key_test(test_memstore())
      .await
      .unwrap()
  }

  #[tokio::test]
  async fn test_memstore_key_generate() {
    StorageTestSuite::key_generate_test(test_memstore()).await.unwrap()
  }

  #[tokio::test]
  async fn test_memstore_key_delete() {
    StorageTestSuite::key_delete_test(test_memstore()).await.unwrap()
  }

  #[tokio::test]
  async fn test_memstore_did_list() {
    StorageTestSuite::did_list_test(test_memstore()).await.unwrap()
  }

  #[tokio::test]
  async fn test_memstore_key_insert() {
    StorageTestSuite::key_insert_test(test_memstore()).await.unwrap()
  }

  #[tokio::test]
  async fn test_memstore_key_sign_ed25519() {
    StorageTestSuite::key_sign_ed25519_test(test_memstore()).await.unwrap()
  }

  #[tokio::test]
  async fn test_memstore_key_value_store() {
    StorageTestSuite::key_value_store_test(test_memstore()).await.unwrap()
  }

  #[tokio::test]
  async fn test_memstore_did_purge() {
    StorageTestSuite::did_purge_test(test_memstore()).await.unwrap()
  }

  #[tokio::test]
  async fn test_memstore_encryption() {
    StorageTestSuite::encryption_test(test_memstore(), test_memstore())
      .await
      .unwrap()
  }
}
