// Copyright 2020-2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;

use identity_stardust::{StardustDID, StardustDocument};
use identity_stardust::block::address::{Address, AliasAddress, Ed25519Address, NftAddress};
use identity_stardust::block::output::AliasOutput;
use identity_stardust::block::output::RentStructure;
use identity_stardust::StardustIdentityClientExt;
use js_sys::Promise;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::error::Result;
use crate::error::WasmResult;
use crate::stardust::{WasmStardustDID, WasmStardustDocument};
use crate::stardust::identity_client::WasmStardustIdentityClient;

// `IAliasOutput` and `IRent` are external interfaces from iota.js.
// See the custom TypeScript section in `identity_client.rs` for the import statement.
#[wasm_bindgen]
extern "C" {
  #[wasm_bindgen(typescript_type = "Promise<IAliasOutput>")]
  pub type PromiseAliasOutput;

  #[wasm_bindgen(typescript_type = "Promise<StardustDocument>")]
  pub type PromiseStardustDocument;

  #[wasm_bindgen(typescript_type = "IRent")]
  pub type IRent;
}

/// An extension interface that provides helper functions for publication
/// and resolution of DID documents in Alias Outputs.
#[wasm_bindgen(js_name = StardustIdentityClientExt)]
pub struct WasmStardustIdentityClientExt;

#[wasm_bindgen(js_class = StardustIdentityClientExt)]
impl WasmStardustIdentityClientExt {
  /// Create a DID with a new Alias Output containing the given `document`.
  ///
  /// The `address` will be set as the state controller and governor unlock conditions.
  /// The minimum required token deposit amount will be set according to the given
  /// `rent_structure`, which will be fetched from the node if not provided.
  /// The returned Alias Output can be further customised before publication, if desired.
  ///
  /// NOTE: this does *not* publish the Alias Output.
  #[allow(non_snake_case)]
  #[wasm_bindgen(js_name = newDidOutput)]
  pub fn new_did_output(
    client: WasmStardustIdentityClient,
    addressKind: u8,
    addressHex: String,
    document: &WasmStardustDocument,
    rentStructure: Option<IRent>,
  ) -> Result<PromiseAliasOutput> {
    // Reconstruct address.
    let address: Address = match addressKind {
      Ed25519Address::KIND => Ed25519Address::from_str(&addressHex).wasm_result()?.into(),
      AliasAddress::KIND => AliasAddress::from_str(&addressHex).wasm_result()?.into(),
      NftAddress::KIND => NftAddress::from_str(&addressHex).wasm_result()?.into(),
      unknown => {
        return Err(identity_stardust::Error::JsError(format!("unknown addressKind {unknown}"))).wasm_result();
      }
    };
    let doc: StardustDocument = document.0.clone();

    let promise: Promise = future_to_promise(async move {
      let rent_structure: Option<RentStructure> = rentStructure.map(|rent| rent.into_serde()).transpose().wasm_result()?;

      let output: AliasOutput = StardustIdentityClientExt::new_did_output(&client, address, doc, rent_structure).await
        .wasm_result()?;
      JsValue::from_serde(&output).wasm_result()
    });

    // WARNING: this does not validate the return type. Check carefully.
    Ok(promise.unchecked_into::<PromiseAliasOutput>())
  }

  /// Fetches the associated Alias Output and updates it with `document` in its state metadata.
  /// The storage deposit on the output is left unchanged. If the size of the document increased,
  /// the amount should be increased manually.
  ///
  /// NOTE: this does *not* publish the updated Alias Output.
  #[wasm_bindgen(js_name = updateDidOutput)]
  pub fn update_did_output(client: WasmStardustIdentityClient, document: &WasmStardustDocument) -> Result<PromiseAliasOutput> {
    let document: StardustDocument = document.0.clone();
    let promise: Promise = future_to_promise(async move {
      let output: AliasOutput = StardustIdentityClientExt::update_did_output(&client, document).await
        .wasm_result()?;
      JsValue::from_serde(&output).wasm_result()
    });

    // WARNING: this does not validate the return type. Check carefully.
    Ok(promise.unchecked_into::<PromiseAliasOutput>())
  }

  /// Resolve a {@link StardustDocument}. Returns an empty, deactivated document if the state metadata
  /// of the Alias Output is empty.
  #[wasm_bindgen(js_name = resolveDid)]
  pub fn resolve_did(client: WasmStardustIdentityClient, did: &WasmStardustDID) -> Result<PromiseStardustDocument> {
    let did: StardustDID = did.0.clone();
    let promise: Promise = future_to_promise(async move {
      StardustIdentityClientExt::resolve_did(&client, &did).await
        .map(WasmStardustDocument)
        .map(Into::into)
        .wasm_result()
    });

    // WARNING: this does not validate the return type. Check carefully.
    Ok(promise.unchecked_into::<PromiseStardustDocument>())
  }

  /// Fetches the `IAliasOutput` associated with the given DID.
  #[wasm_bindgen(js_name = resolveDidOutput)]
  pub fn resolve_did_output(client: WasmStardustIdentityClient, did: &WasmStardustDID) -> Result<PromiseAliasOutput> {
    let did: StardustDID = did.0.clone();
    let promise: Promise = future_to_promise(async move {
      let output: AliasOutput = StardustIdentityClientExt::resolve_did_output(&client, &did).await
        .wasm_result()?;
      JsValue::from_serde(&output).wasm_result()
    });

    // WARNING: this does not validate the return type. Check carefully.
    Ok(promise.unchecked_into::<PromiseAliasOutput>())
  }
}

#[wasm_bindgen(typescript_custom_section)]
const I_STARDUST_IDENTITY_CLIENT_EXT: &'static str = r#"
/** An extension interface that provides helper functions for publication
  * and resolution of DID documents in Alias Outputs.
  */
interface IStardustIdentityClientExt extends IStardustIdentityClient {
  /** Create a DID with a new Alias Output containing the given `document`.
    *
    * The `address` will be set as the state controller and governor unlock conditions.
    * The minimum required token deposit amount will be set according to the given
    * `rent_structure`, which will be fetched from the node if not provided.
    * The returned Alias Output can be further customised before publication, if desired.
    *
    * NOTE: this does *not* publish the Alias Output.
    */
  newDidOutput(addressKind: number, addressHex: string, document: StardustDocument, rentStructure?: IRent): Promise<IAliasOutput>;

  /** Fetches the associated Alias Output and updates it with `document` in its state metadata.
    * The storage deposit on the output is left unchanged. If the size of the document increased,
    * the amount should be increased manually.
    *
    * NOTE: this does *not* publish the updated Alias Output.
    */
  updateDidOutput(document: StardustDocument): Promise<IAliasOutput>;

  /** Resolve a {@link StardustDocument}. Returns an empty, deactivated document if the state
    * metadata of the Alias Output is empty.
    */
  resolveDid(did: StardustDID): Promise<StardustDocument>;

  /** Fetches the `IAliasOutput` associated with the given DID. */
  resolveDidOutput(did: StardustDID): Promise<IAliasOutput>;
}"#;
