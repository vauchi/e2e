// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Production recovery uploads across the real relay and OHTTP forwarder.

use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use vauchi_core::identity::Identity;
use vauchi_core::recovery::{RecoveryVoucher, VerificationResult};
use vauchi_core::{Contact, ContactCard, SymmetricKey, Vauchi, VauchiConfig};

use crate::ohttp_helpers::spawn_ohttp_stack;

// @scenario: contact_recovery :: Recovery succeeds with trusted vouchers only
#[tokio::test(flavor = "multi_thread")]
async fn recovery_wire_uploaded_proof_is_verifiable_by_an_existing_contact() {
    let (mut relays, mut forwarder, relay_url, ohttp_url) = spawn_ohttp_stack().await;
    let anchor = relays.relay(0).unwrap().ohttp_anchor().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = VauchiConfig::with_storage_path(dir.path().join("replacement.db"))
        .with_relay(&relay_url, anchor)
        .with_ohttp_relay_url(ohttp_url);
    let mut replacement = Vauchi::new(config).unwrap();
    replacement.create_identity("Alice replacement").unwrap();
    let old = Identity::create("Alice", 0);
    let claim = replacement
        .create_recovery_claim(old.signing_public_key())
        .unwrap();
    let mut recipient = Vauchi::in_memory().unwrap();
    recipient.create_identity("Recipient").unwrap();
    for name in ["Bob", "Carol", "Dave"] {
        let helper = Identity::create(name, 0);
        let voucher = RecoveryVoucher::create_from_claim(
            &claim,
            helper.signing_keypair(),
            None,
            replacement.clock().unix_seconds(),
        )
        .unwrap();
        replacement
            .add_recovery_voucher(&voucher.to_bytes())
            .unwrap();
        recipient
            .add_contact(Contact::from_exchange(
                *helper.signing_public_key(),
                ContactCard::new(name),
                SymmetricKey::generate(),
                0,
            ))
            .unwrap();
    }
    replacement
        .upload_recovery_proof()
        .expect("upload completed recovery over OHTTP");

    // Inspect the opaque stored payload directly; this is a test observation,
    // not a production client fallback around the OHTTP forwarder.
    let hash = hex::encode(Sha256::digest(old.signing_public_key()));
    let response = reqwest::Client::new()
        .post(format!("{relay_url}/v2/recovery/query"))
        .json(&serde_json::json!({"key_hashes": [hash]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    let proofs = body["proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0]["key_hash"], hash);
    let bytes = STANDARD
        .decode(proofs[0]["proof_data"].as_str().unwrap())
        .unwrap();
    let (proof, confidence) = recipient
        .verify_recovery_proof(&bytes)
        .expect("production upload must use the recipient's recovery-proof wire format");
    assert_eq!(proof.old_pk().as_bytes(), old.signing_public_key());
    assert_eq!(
        proof.new_pk().as_bytes(),
        replacement.identity().unwrap().signing_public_key()
    );
    assert_eq!(proof.voucher_count(), 3);
    match confidence {
        VerificationResult::HighConfidence {
            mutual_vouchers,
            total_vouchers,
        } => {
            assert_eq!(total_vouchers, 3);
            assert_eq!(mutual_vouchers.len(), 3);
        }
        other => panic!("expected confidence from prepared contacts, got {other:?}"),
    }
    assert!(replacement.get_recovery_progress().unwrap().is_none());
    forwarder.stop().await;
    relays.stop_all().await;
}
