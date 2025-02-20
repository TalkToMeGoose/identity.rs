// Copyright 2020-2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use anyhow::Context;
use function_name::named;
use identity_did::did::CoreDID;
use rand::distributions::DistString;
use rand::rngs::OsRng;

use identity_core::convert::FromJson;
use identity_core::convert::ToJson;
use identity_core::crypto::KeyPair;
use identity_core::crypto::KeyType;
use identity_core::crypto::PrivateKey;
use identity_core::crypto::PublicKey;
use identity_iota_core::did::IotaDID;
use identity_iota_core::document::IotaDocument;
use identity_iota_core::document::IotaVerificationMethod;
use identity_iota_core::tangle::MessageId;
use identity_iota_core::tangle::Network;
use identity_iota_core::tangle::NetworkName;

use crate::identity::ChainState;
use crate::types::AgreementInfo;
use crate::types::CekAlgorithm;
use crate::types::DIDType;
use crate::types::EncryptedData;
use crate::types::EncryptionAlgorithm;
use crate::types::KeyLocation;
use crate::types::Signature;

use super::Storage;

macro_rules! ensure {
  ($cond:expr, $($msg:expr),*) => {{
    if !$cond {
      let message: String = format!($( $msg, )*);
      let fn_name: &'static str = function_name!();
      return Err(anyhow::Error::msg(format!("[{}]: {}", fn_name, message)));
    }
  };};
}

macro_rules! ensure_eq {
  ($left:expr, $right:expr, $($msg:expr),*) => {
    ensure!($left == $right, $($msg),*);
  };
}

fn random_string() -> String {
  rand::distributions::Alphanumeric.sample_string(&mut OsRng, 32)
}

/// A test suite for the `Storage` interface.
///
/// This contains a set of tests that a correct storage implementation
/// should pass. Note that not every edge case is tested.
///
/// Tests usually rely on multiple interface methods being implemented, so they should only
/// be run on a fully implemented version. That's why there is not a single test case for every
/// interface method.
pub struct StorageTestSuite;

impl StorageTestSuite {
  #[named]
  pub async fn did_create_private_key_test(storage: impl Storage) -> anyhow::Result<()> {
    let fragment: String = random_string();
    let keypair: KeyPair = KeyPair::new(KeyType::Ed25519).unwrap();
    let network: NetworkName = Network::Mainnet.name();

    let expected_did: CoreDID = IotaDID::new_with_network(keypair.public().as_ref(), network.clone())
      .unwrap()
      .into();
    let expected_location: KeyLocation =
      KeyLocation::new(KeyType::Ed25519, fragment.clone(), keypair.public().as_ref());

    let (did, location): (CoreDID, KeyLocation) = storage
      .did_create(
        DIDType::IotaDID,
        network.clone(),
        &fragment,
        Some(keypair.private().to_owned()),
      )
      .await
      .context("did_create returned an error")?;

    ensure_eq!(
      did,
      expected_did,
      "expected returned did to be `{expected_did}`, was `{did}`"
    );

    ensure_eq!(
      location,
      expected_location,
      "expected returned location to be `{expected_location}`, was `{location}`"
    );

    let exists: bool = storage
      .key_exists(&did, &location)
      .await
      .context("key_exists returned an error")?;

    ensure!(exists, "expected key at location `{location}` to exist");

    let result: Result<_, crate::Error> = storage
      .did_create(DIDType::IotaDID, network, &fragment, Some(keypair.private().to_owned()))
      .await;

    ensure!(
      result.is_err(),
      "expected did_create to return an error when attempting to create an identity from the same private key twice"
    );

    let public_key: PublicKey = storage
      .key_public(&did, &location)
      .await
      .context("key_public returned an error")?;

    ensure_eq!(
      public_key.as_ref(),
      keypair.public().as_ref(),
      "expected key_public to return `{:?}`, returned `{public_key:?}`",
      keypair.public()
    );

    Ok(())
  }

  #[named]
  pub async fn did_create_generate_key_test(storage: impl Storage) -> anyhow::Result<()> {
    let fragment: String = random_string();
    let network: NetworkName = Network::Devnet.name();
    let (core_did, location): (CoreDID, KeyLocation) = storage
      .did_create(DIDType::IotaDID, network.clone(), &fragment, None)
      .await
      .context("did_create returned an error")?;
    let did: IotaDID = IotaDID::try_from(core_did.clone()).unwrap();
    ensure_eq!(
      did.network_str(),
      network.as_ref(),
      "expected network `{network}` for the generated DID, was `{}`",
      did.network_str()
    );

    let exists: bool = storage
      .key_exists(&core_did, &location)
      .await
      .context("key_exists returned an error")?;

    ensure!(exists, "expected key at location `{location}` to exist");

    let public_key: PublicKey = storage
      .key_public(&core_did, &location)
      .await
      .context("key_public returned an error")?;

    let expected_did: IotaDID = IotaDID::new_with_network(public_key.as_ref(), network).unwrap();

    ensure_eq!(
    did,
    expected_did,
    "returned did `{did}` did not match did created from retrieved public key and network, expected: `{expected_did}`"
  );

    Ok(())
  }

  #[named]
  pub async fn key_generate_test(storage: impl Storage) -> anyhow::Result<()> {
    let fragment: String = random_string();
    let network: NetworkName = Network::Mainnet.name();

    let (did, _): (CoreDID, _) = storage
      .did_create(DIDType::IotaDID, network.clone(), &fragment, None)
      .await
      .context("did_create returned an error")?;

    let key_types: [KeyType; 2] = [KeyType::Ed25519, KeyType::X25519];

    let mut locations: Vec<KeyLocation> = Vec::with_capacity(key_types.len());

    for key_type in key_types {
      let key_fragment: String = random_string();
      let location: KeyLocation = storage
        .key_generate(&did, key_type, &key_fragment)
        .await
        .context("key_generate returned an error")?;
      locations.push(location);
    }

    for location in locations {
      let exists: bool = storage
        .key_exists(&did, &location)
        .await
        .context("key_exists returned an error")?;

      ensure!(exists, "expected key at location `{location}` to exist");

      // Ensure we can retrieve the public key without erroring.
      storage
        .key_public(&did, &location)
        .await
        .context("key_public returned an error")?;
    }

    Ok(())
  }

  #[named]
  pub async fn key_delete_test(storage: impl Storage) -> anyhow::Result<()> {
    const NUM_IDENTITIES: usize = 20;
    let fragment: String = random_string();
    let network: NetworkName = Network::Mainnet.name();

    let (did, _): (CoreDID, _) = storage
      .did_create(DIDType::IotaDID, network.clone(), &fragment, None)
      .await
      .context("did_create returned an error")?;

    let mut locations: Vec<KeyLocation> = Vec::with_capacity(NUM_IDENTITIES);

    for _ in 0..NUM_IDENTITIES {
      let key_fragment: String = random_string();
      let location: KeyLocation = storage
        .key_generate(&did, KeyType::Ed25519, &key_fragment)
        .await
        .context("key_generate returned an error")?;
      locations.push(location);
    }

    for location in locations {
      let exists: bool = storage
        .key_exists(&did, &location)
        .await
        .context("key_exists returned an error")?;

      ensure!(exists, "expected key at location `{location}` to exist");

      let deleted: bool = storage
        .key_delete(&did, &location)
        .await
        .context("key_delete returned an error")?;

      ensure!(deleted, "expected key at location `{location}` to be deleted");

      let deleted: bool = storage
        .key_delete(&did, &location)
        .await
        .context("key_delete returned an error")?;

      ensure!(!deleted, "expected key at location `{location}` to already be deleted");
    }

    Ok(())
  }

  #[named]
  pub async fn did_list_test(storage: impl Storage) -> anyhow::Result<()> {
    const NUM_IDENTITIES: usize = 20;
    let fragment: String = random_string();
    let network: NetworkName = Network::Mainnet.name();

    let list: Vec<CoreDID> = storage.did_list().await.context("did_list returned an error")?;

    ensure!(
      list.is_empty(),
      "expected list to be empty, but found {} element(s)",
      list.len()
    );

    for i in 0..NUM_IDENTITIES {
      let (did, _): (CoreDID, _) = storage
        .did_create(DIDType::IotaDID, network.clone(), &fragment, None)
        .await
        .context("did_create returned an error")?;

      let exists: bool = storage.did_exists(&did).await.context("did_exists returned an error")?;
      ensure!(exists, "expected did `{did}` to exist");

      let list_len: usize = storage.did_list().await.context("did_list returned an error")?.len();
      let expected_len: usize = i + 1;

      ensure_eq!(
        list_len,
        expected_len,
        "expected did_list to return a list of len {expected_len}, got {list_len} elements instead"
      );
    }

    Ok(())
  }

  #[named]
  pub async fn key_insert_test(storage: impl Storage) -> anyhow::Result<()> {
    let fragment: String = random_string();
    let network: NetworkName = Network::Mainnet.name();

    let (did, _): (CoreDID, _) = storage
      .did_create(DIDType::IotaDID, network.clone(), &fragment, None)
      .await
      .context("did_create returned an error")?;

    let key_types: [KeyType; 2] = [KeyType::Ed25519, KeyType::X25519];

    let mut locations: Vec<KeyLocation> = Vec::with_capacity(key_types.len());
    let mut public_keys: Vec<PublicKey> = Vec::with_capacity(key_types.len());

    for key_type in key_types {
      let key_fragment: String = random_string();
      let keypair: KeyPair = KeyPair::new(key_type).unwrap();
      let location: KeyLocation = KeyLocation::new(key_type, key_fragment, keypair.public().as_ref());

      storage
        .key_insert(&did, &location, keypair.private().to_owned())
        .await
        .context("key_insert returned an error")?;

      public_keys.push(keypair.public().to_owned());
      locations.push(location);
    }

    for (i, location) in locations.into_iter().enumerate() {
      let exists: bool = storage
        .key_exists(&did, &location)
        .await
        .context("key_exists returned an error")?;

      ensure!(exists, "expected key at location `{location}` to exist");

      let public_key: PublicKey = storage
        .key_public(&did, &location)
        .await
        .context("key_public returned an error")?;

      let expected_public_key: &PublicKey = &public_keys[i];

      ensure_eq!(
        public_key.as_ref(),
        expected_public_key.as_ref(),
        "expected public key at location `{location}` to be {expected_public_key:?}, was {public_key:?}"
      );
    }

    Ok(())
  }

  #[named]
  pub async fn key_sign_ed25519_test(storage: impl Storage) -> anyhow::Result<()> {
    // The following test vector is taken from Test 2 of RFC 8032
    // https://datatracker.ietf.org/doc/html/rfc8032#section-7
    const PRIVATE_KEY: [u8; 32] = [
      76, 205, 8, 155, 40, 255, 150, 218, 157, 182, 195, 70, 236, 17, 78, 15, 91, 138, 49, 159, 53, 171, 166, 36, 218,
      140, 246, 237, 79, 184, 166, 251,
    ];
    const MESSAGE: [u8; 1] = [114];
    const SIGNATURE: [u8; 64] = [
      146, 160, 9, 169, 240, 212, 202, 184, 114, 14, 130, 11, 95, 100, 37, 64, 162, 178, 123, 84, 22, 80, 63, 143, 179,
      118, 34, 35, 235, 219, 105, 218, 8, 90, 193, 228, 62, 21, 153, 110, 69, 143, 54, 19, 208, 241, 29, 140, 56, 123,
      46, 174, 180, 48, 42, 238, 176, 13, 41, 22, 18, 187, 12, 0,
    ];

    let fragment: String = random_string();
    let network: NetworkName = Network::Mainnet.name();

    let (did, location): (CoreDID, KeyLocation) = storage
      .did_create(
        DIDType::IotaDID,
        network.clone(),
        &fragment,
        Some(PrivateKey::from(PRIVATE_KEY.to_vec())),
      )
      .await
      .context("did_create returned an error")?;

    let signature: Signature = storage
      .key_sign(&did, &location, MESSAGE.to_vec())
      .await
      .context("key_sign returned an error")?;

    ensure_eq!(
      signature.as_bytes(),
      &SIGNATURE,
      "expected signature to be `{SIGNATURE:?}`, was `{:?}`",
      signature.as_bytes()
    );

    Ok(())
  }

  #[named]
  pub async fn key_value_store_test(storage: impl Storage) -> anyhow::Result<()> {
    let fragment: String = random_string();
    let network: NetworkName = Network::Mainnet.name();

    let (did, location): (CoreDID, KeyLocation) = storage
      .did_create(DIDType::IotaDID, network.clone(), &fragment, None)
      .await
      .context("did_create returned an error")?;

    let value: Option<Vec<u8>> = storage.blob_get(&did).await.context("blob_get returned an error")?;

    ensure!(value.is_none(), "expected blob_get to return `None` for a new DID");

    let public_key: PublicKey = storage
      .key_public(&did, &location)
      .await
      .context("key_public returned an error")?;

    let method: IotaVerificationMethod = IotaVerificationMethod::new(
      did.clone().try_into().unwrap(),
      KeyType::Ed25519,
      &public_key,
      &fragment,
    )
    .unwrap();

    let expected_document: IotaDocument = IotaDocument::from_verification_method(method).unwrap();
    storage
      .blob_set(&did, expected_document.to_json_vec().unwrap())
      .await
      .context("blob_set returned an error")?;
    let value: Option<Vec<u8>> = storage.blob_get(&did).await.context("blob_get returned an error")?;
    let document: IotaDocument = IotaDocument::from_json_slice(&value.unwrap()).unwrap();
    ensure_eq!(
      expected_document,
      document,
      "expected `{expected_document}`, got `{document}`"
    );

    let mut expected_chain_state: ChainState = ChainState::new();
    expected_chain_state.set_last_integration_message_id(MessageId::new([0xff; 32]));
    storage
      .blob_set(&did, expected_chain_state.to_json_vec().unwrap())
      .await
      .context("blob_set returned an error")?;
    let value: Option<Vec<u8>> = storage.blob_get(&did).await.context("blob_get returned an error")?;
    let chain_state: ChainState = ChainState::from_json_slice(&value.unwrap()).unwrap();
    ensure_eq!(
      expected_chain_state,
      chain_state,
      "expected `{expected_chain_state:?}`, got `{chain_state:?}`"
    );

    Ok(())
  }

  #[named]
  pub async fn did_purge_test(storage: impl Storage) -> anyhow::Result<()> {
    let fragment: String = random_string();
    let network: NetworkName = Network::Mainnet.name();

    let (did, location): (CoreDID, KeyLocation) = storage
      .did_create(DIDType::IotaDID, network.clone(), &fragment, None)
      .await
      .context("did_create returned an error")?;

    let list_len: usize = storage.did_list().await.context("did_list returned an error")?.len();

    ensure_eq!(
      list_len,
      1,
      "expected did_list to return a list of size 1 after creation"
    );

    let mut expected_chain_state: ChainState = ChainState::new();
    expected_chain_state.set_last_integration_message_id(MessageId::new([0xff; 32]));

    storage
      .blob_set(&did, expected_chain_state.to_json_vec().unwrap())
      .await
      .context("chain_state_set returned an error")?;

    let purged: bool = storage.did_purge(&did).await.context("did_purge returned an error")?;

    ensure!(purged, "expected did `{did}` to have been purged");

    let value: Option<Vec<u8>> = storage.blob_get(&did).await.context("blob_get returned an error")?;

    ensure!(value.is_none(), "expected blob_get to return `None` after purging");

    let exists: bool = storage
      .key_exists(&did, &location)
      .await
      .context("key_exists returned an error")?;

    ensure!(
      !exists,
      "expected key at location `{location}` to no longer exist after purge"
    );

    let list: Vec<CoreDID> = storage.did_list().await.context("did_list returned an error")?;

    ensure!(
      list.is_empty(),
      "expected did_list to return an empty list after purging"
    );

    Ok(())
  }

  #[named]
  pub async fn encryption_test(alice_storage: impl Storage, bob_storage: impl Storage) -> anyhow::Result<()> {
    let agreement: AgreementInfo = AgreementInfo::new(b"Alice".to_vec(), b"Bob".to_vec(), Vec::new(), Vec::new());

    for cek_algorithm in [
      CekAlgorithm::ECDH_ES(agreement.clone()),
      CekAlgorithm::ECDH_ES_A256KW(agreement),
    ] {
      let network: NetworkName = Network::Mainnet.name();

      // Both Alice (Sender) and Bob (Receiver) must have a DID.
      let (alice_did, _): (CoreDID, KeyLocation) = alice_storage
        .did_create(DIDType::IotaDID, network.clone(), &random_string(), None)
        .await
        .context("did_create returned an error")?;

      let (bob_did, _): (CoreDID, KeyLocation) = bob_storage
        .did_create(DIDType::IotaDID, network.clone(), &random_string(), None)
        .await
        .context("did_create returned an error")?;

      // The target of the message must share an X25519 public key.
      let bob_fragment: String = random_string();
      let bob_location: KeyLocation = bob_storage
        .key_generate(&bob_did, KeyType::X25519, &bob_fragment)
        .await
        .context("key_generate returned an error")?;
      let bob_public_key: PublicKey = bob_storage
        .key_public(&bob_did, &bob_location)
        .await
        .context("key_public returned an error")?;

      // Alice encrypts the message to be sent to Bob.
      let encryption_algorithm: EncryptionAlgorithm = EncryptionAlgorithm::AES256GCM;
      let plaintext: &[u8] = b"This msg will be encrypted and decrypted";

      let encrypted_data: EncryptedData = alice_storage
        .data_encrypt(
          &alice_did,
          plaintext.to_vec(),
          b"associated_data".to_vec(),
          &encryption_algorithm,
          &cek_algorithm,
          bob_public_key,
        )
        .await
        .context("data_encrypt returned an error")?;

      // Bob must be able to decrypt the message using the shared secret.
      let decrypted_msg: Vec<u8> = bob_storage
        .data_decrypt(
          &bob_did,
          encrypted_data,
          &encryption_algorithm,
          &cek_algorithm,
          &bob_location,
        )
        .await
        .context("data_decrypt returned an error")?;

      ensure_eq!(
        plaintext,
        &decrypted_msg,
        "decrypted message does not match the original message"
      );
    }

    Ok(())
  }
}
