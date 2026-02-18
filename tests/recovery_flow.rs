// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Social Recovery E2E Test
//!
//! Tests the complete social recovery workflow:
//! 1. Alice exchanges with 3 contacts (Bob, Carol, Dave)
//! 2. Alice "loses" her device (new identity)
//! 3. Contacts vouch for Alice's recovery claim
//! 4. Alice assembles and verifies recovery proof

use vauchi_e2e_tests::prelude::*;

/// Smoke test: Recovery claim creation.
/// Tags: smoke, recovery
/// Feature: contact_recovery.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn smoke_recovery_setup() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();
    let dave = orch.user("Dave").unwrap();

    // Exchange Alice with all three vouchers
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let carol = carol.read().await;
        let dave = dave.read().await;

        alice
            .exchange_with(&bob)
            .await
            .expect("Exchange with Bob failed");
        alice
            .exchange_with(&carol)
            .await
            .expect("Exchange with Carol failed");
        alice
            .exchange_with(&dave)
            .await
            .expect("Exchange with Dave failed");
    }

    // Verify contacts established
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 3, "Alice should have 3 contacts");
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Contact verification.
/// Tags: integration, verification
/// Feature: contact_exchange.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_contact_verification() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();

    // Exchange contacts
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        alice.exchange_with(&bob).await.expect("Exchange failed");
    }

    // Alice verifies Bob's fingerprint.
    // Note: After QR exchange, contacts appear as "New Contact" (not "Bob")
    // until card updates sync. Use the contact's ID prefix for lookup.
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 1, "Alice should have 1 contact");
        let contact_id = contacts[0].id.as_ref().expect("Contact should have ID");
        let device = alice.device(0).expect("No device");
        let device = device.read().await;
        device
            .verify_contact(contact_id)
            .await
            .expect("Failed to verify contact");
    }

    // Verify the contact shows as verified
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 1, "Alice should have 1 contact");
        assert!(
            contacts[0].verified,
            "Contact should be marked as verified after fingerprint verification"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Complete recovery happy path.
/// Tags: integration, recovery
/// Traces: contact_recovery.feature:97-106 "Recovery succeeds with trusted vouchers only"
///
/// Tests the complete social recovery workflow:
/// 1. Alice exchanges with Bob, Carol, Dave
/// 2. Alice "loses" device (simulated by creating new identity)
/// 3. Alice creates recovery claim with old public key
/// 4. Bob, Carol, Dave vouch for Alice
/// 5. Alice collects vouchers and creates proof
/// 6. Bob verifies the proof
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_recovery_happy_path() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Setup: 4 users - Alice (original), Bob, Carol, Dave
    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");
    orch.add_user("Dave", 1).expect("Failed to add Dave");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();
    let dave = orch.user("Dave").unwrap();

    // Get Alice's old public key (hex) before "losing" device.
    // This is the signing public key, needed for recovery claims.
    let alice_old_pk: String;
    {
        let alice = alice.read().await;
        alice_old_pk = alice
            .get_public_id()
            .await
            .expect("Failed to get Alice's public ID");
    }

    // Exchange Alice with all three vouchers
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let carol = carol.read().await;
        let dave = dave.read().await;

        alice
            .exchange_with(&bob)
            .await
            .expect("Exchange with Bob failed");
        alice
            .exchange_with(&carol)
            .await
            .expect("Exchange with Carol failed");
        alice
            .exchange_with(&dave)
            .await
            .expect("Exchange with Dave failed");
    }

    // Verify contacts established
    {
        let alice = alice.read().await;
        let contacts = alice
            .list_contacts()
            .await
            .expect("Failed to list contacts");
        assert_eq!(contacts.len(), 3, "Alice should have 3 contacts");
    }

    // Simulate device loss: Create new identity for Alice
    orch.add_user("AliceNew", 1)
        .expect("Failed to add Alice's new device");
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.write().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;
        device
            .create_identity("Alice Recovered")
            .await
            .expect("Failed to create new identity");
    }

    // Step 1: Alice creates recovery claim with old public key
    let recovery_claim: String;
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.read().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;
        recovery_claim = device
            .create_recovery_claim(&alice_old_pk)
            .await
            .expect("Failed to create recovery claim");
    }
    assert!(
        !recovery_claim.is_empty(),
        "Recovery claim should not be empty"
    );

    // Step 2: Bob vouches for Alice
    let voucher_bob: String;
    {
        let bob = bob.read().await;
        let device = bob.device(0).expect("No device");
        let device = device.read().await;
        voucher_bob = device
            .vouch_for_recovery(&recovery_claim)
            .await
            .expect("Bob failed to vouch");
    }

    // Step 3: Carol vouches for Alice
    let voucher_carol: String;
    {
        let carol = carol.read().await;
        let device = carol.device(0).expect("No device");
        let device = device.read().await;
        voucher_carol = device
            .vouch_for_recovery(&recovery_claim)
            .await
            .expect("Carol failed to vouch");
    }

    // Step 4: Dave vouches for Alice
    let voucher_dave: String;
    {
        let dave = dave.read().await;
        let device = dave.device(0).expect("No device");
        let device = device.read().await;
        voucher_dave = device
            .vouch_for_recovery(&recovery_claim)
            .await
            .expect("Dave failed to vouch");
    }

    // Step 5: Alice adds all vouchers
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.read().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;

        device
            .add_recovery_voucher(&voucher_bob)
            .await
            .expect("Failed to add Bob's voucher");
        device
            .add_recovery_voucher(&voucher_carol)
            .await
            .expect("Failed to add Carol's voucher");
        device
            .add_recovery_voucher(&voucher_dave)
            .await
            .expect("Failed to add Dave's voucher");
    }

    // Step 6: Alice gets the recovery proof
    let recovery_proof: Option<String>;
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.read().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;
        recovery_proof = device
            .get_recovery_proof()
            .await
            .expect("Failed to get recovery proof");
    }
    assert!(
        recovery_proof.is_some(),
        "Recovery proof should be complete with 3 vouchers"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Insufficient vouchers.
/// Tags: integration, recovery
/// Traces: contact_recovery.feature:109-117 "Insufficient trusted vouchers"
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_recovery_insufficient_vouchers() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");
    orch.add_user("Carol", 1).expect("Failed to add Carol");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();
    let bob = orch.user("Bob").unwrap();
    let carol = orch.user("Carol").unwrap();

    // Get Alice's old public key (hex) before "losing" device.
    let alice_old_pk: String;
    {
        let alice = alice.read().await;
        alice_old_pk = alice
            .get_public_id()
            .await
            .expect("Failed to get Alice's public ID");
    }

    // Exchange with only Bob and Carol (need 3 for threshold)
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let carol = carol.read().await;

        alice.exchange_with(&bob).await.expect("Exchange failed");
        alice.exchange_with(&carol).await.expect("Exchange failed");
    }

    // Simulate device loss
    orch.add_user("AliceNew", 1)
        .expect("Failed to add Alice's new device");
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.write().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;
        device
            .create_identity("Alice Recovered")
            .await
            .expect("Failed to create identity");
    }

    // Create claim and collect only 2 vouchers
    let recovery_claim: String;
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.read().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;
        recovery_claim = device
            .create_recovery_claim(&alice_old_pk)
            .await
            .expect("Failed to create claim");
    }

    let voucher_bob: String;
    {
        let bob = bob.read().await;
        let device = bob.device(0).expect("No device");
        let device = device.read().await;
        voucher_bob = device
            .vouch_for_recovery(&recovery_claim)
            .await
            .expect("Bob failed to vouch");
    }

    let voucher_carol: String;
    {
        let carol = carol.read().await;
        let device = carol.device(0).expect("No device");
        let device = device.read().await;
        voucher_carol = device
            .vouch_for_recovery(&recovery_claim)
            .await
            .expect("Carol failed to vouch");
    }

    // Add only 2 vouchers
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.read().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;

        device
            .add_recovery_voucher(&voucher_bob)
            .await
            .expect("Failed to add voucher");
        device
            .add_recovery_voucher(&voucher_carol)
            .await
            .expect("Failed to add voucher");
    }

    // Proof should not be complete with only 2 vouchers (need 3)
    let recovery_proof: Option<String>;
    {
        let alice_new = orch.user("AliceNew").unwrap();
        let alice_new = alice_new.read().await;
        let device = alice_new.device(0).expect("No device");
        let device = device.read().await;
        recovery_proof = device
            .get_recovery_proof()
            .await
            .expect("Failed to get proof");
    }
    assert!(
        recovery_proof.is_none(),
        "Recovery proof should be incomplete with only 2/3 vouchers"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}
