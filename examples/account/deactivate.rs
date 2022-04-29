// Copyright 2020-2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! cargo run --example account_deactivate

use identity::account::Account;
use identity::account::AccountBuilder;
use identity::account::IdentitySetup;
use identity::account::Result;
use identity::iota::DocumentHistory;
use identity::iota::Resolver;
use identity::iota_core::IotaDID;
use identity::prelude::*;

pub async fn run() -> Result<()> {
  // Create an account builder with in-memory storage for simplicity.
  // See `create_did` example to configure Stronghold storage.
  let mut builder: AccountBuilder = Account::builder();

  // Create a new identity managed by an account
  let mut account: Account = builder.create_identity(IdentitySetup::default()).await?;

  println!(
    "Resolved DID Document directly after creation: {:#?}",
    account.resolve_identity().await?
  );

  /*
  Uncommenting the following code will give an error saying "Error: UpdateError(InvalidMethodFragment("cannot remove last signing method"))":
  account
  .update_identity()
  .delete_method()
  .fragment("sign-0")
  .apply()
  .await?;
   */

  // To delete the last signing method we need to manipulate the DID Document directly
  let current_document: &IotaDocument = account.document();
  let mut new_document: IotaDocument = current_document.clone();

  // Delete the last signing method from the cloned DID Document
  new_document.remove_method(current_document.default_signing_method()?.id())?;

  // Force the account to overwrite the managed DID Document with the new DID Document
  account.update_document_unchecked(new_document).await?;

  println!(
    "Resolved DID Document after removing the last verification method: {:#?}",
    account.resolve_identity().await?
  );

  // The last signing method has been deleted from the Tangle. Now we also remove the identity from the storage.

  // Retain the DID before deleting the account.
  let did: IotaDID = account.did().clone();
  account.delete_identity().await?;

  // Resolve the history of the DID Document.
  let resolver: Resolver = Resolver::new().await?;

  let history: DocumentHistory = resolver.resolve_history(&did).await?;
  println!("History of the DID Document: {:#?}", history);

  Ok(())
}

#[allow(dead_code)]
#[tokio::main]
async fn main() -> Result<()> {
  let _ = run().await?;

  Ok(())
}
