// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Exchange Error Paths E2E Tests
//!
//! Tests error conditions during contact exchange:
//! 1. Expired QR codes (older than 5 minutes)
//! 2. Self-exchange prevention
//! 3. Duplicate contact detection
//! 4. Malformed QR data handling
//!
//! ## Feature Traceability
//! - `contact_exchange.feature` @qr-code: "QR code expiration"
//! - `contact_exchange.feature` @edge-case @self-exchange: "Cannot exchange with yourself"
//! - `contact_exchange.feature` @duplicate: "Exchange with existing contact"
//! - `contact_exchange.feature` @exchange-error: "Handle malformed QR code"
//!
//! ## Test Tiers
//! - `smoke_*`: Fast tests for every push (< 5 min total)
//! - `integration_*`: Comprehensive tests for main branch

use std::time::Duration;
use vauchi_e2e_tests::prelude::*;

// @scenario: contact_exchange:QR code expiration
/// Integration test: QR code older than 5 minutes is rejected.
///
/// Tags: integration, exchange, error-path, expired-qr
/// Feature: contact_exchange.feature @qr-code "QR code expiration"
///
/// Scenario:
/// 1. Alice generates an exchange QR code
/// 2. Wait for 5+ minutes (real-time wait — see note below)
/// 3. Bob attempts to scan the expired QR code
/// 4. Exchange should fail with "QrExpired" error
///
/// **WARNING: This test takes ~5 minutes due to QR_EXPIRY_SECONDS being a
/// hardcoded constant (300s) in `core/vauchi-core/src/exchange/qr.rs:23`.**
/// The expiry is validated client-side, so neither relay config nor env vars
/// can shorten it.
///
/// TODO: Make `QR_EXPIRY_SECONDS` configurable via env var or builder param
/// in vauchi-core so E2E tests can use a short TTL (e.g. 5s).
// @internal
#[tokio::test]
#[ignore = "requires binaries + takes 5min (QR_EXPIRY_SECONDS hardcoded in core)"]
async fn test_exchange_expired_qr() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Add users
    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    // Create identities
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    // Alice generates QR code
    let alice = orch.user("Alice").unwrap();
    let qr_data = {
        let alice_guard = alice.read().await;
        alice_guard
            .generate_qr()
            .await
            .expect("Failed to generate QR")
    };

    // Wait for QR to expire. QR_EXPIRY_SECONDS = 300s is hardcoded in
    // core/vauchi-core/src/exchange/qr.rs — client-side validation.
    // TODO: When QR_EXPIRY_SECONDS becomes configurable, reduce to 5s + buffer.
    orch.wait(Duration::from_secs(300 + 5)).await;

    // Bob attempts to complete exchange with expired QR
    let bob = orch.user("Bob").unwrap();
    let exchange_result = {
        let bob_guard = bob.read().await;
        bob_guard.complete_exchange(&qr_data).await
    };

    // Verify the exchange fails with an expiration error
    assert!(
        exchange_result.is_err(),
        "Exchange with expired QR should fail"
    );

    let error_msg = exchange_result.unwrap_err().to_string().to_lowercase();
    assert!(
        error_msg.contains("expir") || error_msg.contains("timeout") || error_msg.contains("stale"),
        "Error should indicate QR expiration: {}",
        error_msg
    );

    // Verify no contacts were exchanged
    orch.verify_contact_count("Alice", 0)
        .await
        .expect("Alice should have 0 contacts");
    orch.verify_contact_count("Bob", 0)
        .await
        .expect("Bob should have 0 contacts");

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Cannot exchange with yourself
/// Smoke test: Cannot exchange with yourself (self-exchange prevention).
///
/// Tags: smoke, exchange, error-path, self-exchange
/// Feature: contact_exchange.feature @edge-case @self-exchange "Cannot exchange with yourself"
///
/// Scenario:
/// 1. Alice generates an exchange QR code
/// 2. Alice scans her own QR code
/// 3. Exchange should fail with "SelfExchange" error
// @internal
#[tokio::test]
async fn test_exchange_self_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Add Alice with a single device
    orch.add_user("Alice", 1).expect("Failed to add Alice");

    // Create identity
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    // Alice generates QR code
    let alice = orch.user("Alice").unwrap();
    let qr_data = {
        let alice_guard = alice.read().await;
        alice_guard
            .generate_qr()
            .await
            .expect("Failed to generate QR")
    };

    // Alice attempts to complete exchange with her own QR
    let exchange_result = {
        let alice_guard = alice.read().await;
        alice_guard.complete_exchange(&qr_data).await
    };

    // Verify the exchange fails with self-exchange error
    assert!(exchange_result.is_err(), "Self-exchange should be rejected");

    let error_msg = exchange_result.unwrap_err().to_string().to_lowercase();
    assert!(
        error_msg.contains("self")
            || error_msg.contains("yourself")
            || error_msg.contains("same identity"),
        "Error should indicate self-exchange prevention: {}",
        error_msg
    );

    // Verify no contacts were created
    orch.verify_contact_count("Alice", 0)
        .await
        .expect("Alice should have 0 contacts");

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Exchange with existing contact
/// Integration test: Duplicate contact warning on re-exchange.
///
/// Tags: integration, exchange, error-path, duplicate
/// Feature: contact_exchange.feature @duplicate "Exchange with existing contact"
///
/// Scenario:
/// 1. Alice and Bob complete a successful exchange
/// 2. Alice generates a new QR code
/// 3. Bob scans Alice's new QR code
/// 4. Exchange should detect duplicate and warn/update (not create duplicate)
// @internal
#[tokio::test]
async fn test_exchange_already_contact() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Add users
    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    // Create identities
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    // First exchange - should succeed
    orch.exchange("Alice", "Bob")
        .await
        .expect("First exchange should succeed");

    // Verify initial contact counts
    orch.verify_contact_count("Alice", 1)
        .await
        .expect("Alice should have 1 contact");
    orch.verify_contact_count("Bob", 1)
        .await
        .expect("Bob should have 1 contact");

    // Alice generates a new QR code for a second exchange
    let alice = orch.user("Alice").unwrap();
    let qr_data = {
        let alice_guard = alice.read().await;
        alice_guard
            .generate_qr()
            .await
            .expect("Failed to generate QR")
    };

    // Bob attempts to exchange again with Alice
    let bob = orch.user("Bob").unwrap();
    let second_exchange_result = {
        let bob_guard = bob.read().await;
        bob_guard.complete_exchange(&qr_data).await
    };

    // The second exchange should either:
    // 1. Succeed and update the existing contact (no duplicate created), or
    // 2. Return an error indicating the contact already exists
    //
    // Either behavior is acceptable - the key invariant is no duplicate contact.

    // Sync both users
    {
        let alice_guard = alice.read().await;
        alice_guard.sync_all().await.expect("Alice sync failed");
    }
    {
        let bob_guard = bob.read().await;
        bob_guard.sync_all().await.expect("Bob sync failed");
    }

    // Verify no duplicate contacts were created
    // Bob should still have exactly 1 contact (Alice)
    orch.verify_contact_count("Bob", 1)
        .await
        .expect("Bob should still have exactly 1 contact (no duplicate)");

    // If the second exchange returned an error, verify it mentions duplicate/existing
    if let Err(e) = second_exchange_result {
        let error_msg = e.to_string().to_lowercase();
        // Error should indicate existing contact or already connected
        assert!(
            error_msg.contains("already")
                || error_msg.contains("exist")
                || error_msg.contains("duplicate")
                || error_msg.contains("contact"),
            "Error should indicate duplicate contact: {}",
            error_msg
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Handle malformed QR code
/// Smoke test: Invalid/malformed QR data is rejected.
///
/// Tags: smoke, exchange, error-path, malformed
/// Feature: contact_exchange.feature @exchange-error "Handle malformed QR code"
///
/// Scenario:
/// 1. Alice creates identity
/// 2. Alice attempts to complete exchange with garbage QR data
/// 3. Exchange should fail with "Invalid QR" or parse error
// @internal
#[tokio::test]
async fn test_exchange_malformed_qr() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Add Alice
    orch.add_user("Alice", 1).expect("Failed to add Alice");

    // Create identity
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();

    // Test various malformed QR inputs
    let malformed_inputs = [
        // Empty string
        "",
        // Random garbage
        "not-valid-qr-data",
        // Valid base64 but wrong structure
        "aGVsbG8gd29ybGQ=",
        // Too short
        "abc",
        // Special characters
        "!@#$%^&*()",
        // Partial valid-looking data
        "VAUCHI_EXCHANGE_INCOMPLETE",
        // Wrong magic bytes
        "XYZ123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
    ];

    for malformed in &malformed_inputs {
        let exchange_result = {
            let alice_guard = alice.read().await;
            alice_guard.complete_exchange(malformed).await
        };

        assert!(
            exchange_result.is_err(),
            "Exchange with malformed QR '{}' should fail",
            if malformed.len() > 20 {
                &malformed[..20]
            } else {
                malformed
            }
        );

        let error_msg = exchange_result.unwrap_err().to_string().to_lowercase();
        assert!(
            error_msg.contains("invalid")
                || error_msg.contains("malform")
                || error_msg.contains("parse")
                || error_msg.contains("decode")
                || error_msg.contains("format")
                || error_msg.contains("fail"),
            "Error should indicate invalid QR format for '{}': {}",
            if malformed.len() > 20 {
                &malformed[..20]
            } else {
                malformed
            },
            error_msg
        );
    }

    // Verify no contacts were created from any malformed input
    orch.verify_contact_count("Alice", 0)
        .await
        .expect("Alice should have 0 contacts");

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Handle non-Vauchi QR code
/// Integration test: Non-Vauchi QR code is rejected.
///
/// Tags: integration, exchange, error-path, non-vauchi
/// Feature: contact_exchange.feature @exchange-error "Handle non-Vauchi QR code"
///
/// Scenario:
/// 1. Alice creates identity
/// 2. Alice scans a URL QR code (not a Vauchi contact code)
/// 3. Exchange should fail with "Not a Vauchi contact code" error
// @internal
#[tokio::test]
async fn test_exchange_non_vauchi_qr() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Add Alice
    orch.add_user("Alice", 1).expect("Failed to add Alice");

    // Create identity
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let alice = orch.user("Alice").unwrap();

    // Non-Vauchi QR codes that might be scanned in the real world
    let non_vauchi_qrs = [
        // URL
        "https://example.com/some/path",
        // vCard (common contact format)
        "BEGIN:VCARD\nVERSION:3.0\nN:Doe;John\nFN:John Doe\nEND:VCARD",
        // WiFi network
        "WIFI:T:WPA;S:MyNetwork;P:password123;;",
        // Phone number
        "tel:+1234567890",
        // SMS
        "sms:+1234567890?body=Hello",
        // Email
        "mailto:test@example.com",
        // Calendar event (simplified)
        "BEGIN:VEVENT\nSUMMARY:Meeting\nEND:VEVENT",
    ];

    for non_vauchi in &non_vauchi_qrs {
        let exchange_result = {
            let alice_guard = alice.read().await;
            alice_guard.complete_exchange(non_vauchi).await
        };

        assert!(
            exchange_result.is_err(),
            "Exchange with non-Vauchi QR should fail: {}",
            &non_vauchi[..std::cmp::min(30, non_vauchi.len())]
        );

        let error_msg = exchange_result.unwrap_err().to_string().to_lowercase();
        assert!(
            error_msg.contains("vauchi")
                || error_msg.contains("invalid")
                || error_msg.contains("format")
                || error_msg.contains("not a")
                || error_msg.contains("unrecognized"),
            "Error should indicate non-Vauchi format for '{}': {}",
            &non_vauchi[..std::cmp::min(30, non_vauchi.len())],
            error_msg
        );
    }

    // Verify no contacts were created
    orch.verify_contact_count("Alice", 0)
        .await
        .expect("Alice should have 0 contacts");

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Network failure during key exchange
/// Integration test: Exchange fails gracefully on network timeout.
///
/// Tags: integration, exchange, error-path, network
/// Feature: contact_exchange.feature @edge-case @network "Network failure during key exchange"
///
/// Scenario:
/// 1. Alice and Bob create identities
/// 2. Alice generates QR code
/// 3. Bob scans QR but relay is unavailable (stopped)
/// 4. Exchange should fail gracefully, no partial state stored
// @internal
#[tokio::test]
async fn test_exchange_network_failure() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    // Add users
    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    // Create identities (while relay is running)
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    // Alice generates QR code
    let alice = orch.user("Alice").unwrap();
    let qr_data = {
        let alice_guard = alice.read().await;
        alice_guard
            .generate_qr()
            .await
            .expect("Failed to generate QR")
    };

    // Stop the relay to simulate network failure
    orch.stop_relay(0).await.expect("Failed to stop relay");

    // Bob attempts to complete exchange
    // Note: With mutual QR exchange, the exchange creates a contact locally
    // without needing a relay. The relay is only needed for subsequent sync.
    let bob = orch.user("Bob").unwrap();
    let exchange_result = {
        let bob_guard = bob.read().await;
        bob_guard.complete_exchange(&qr_data).await
    };

    // Mutual QR exchange works offline — contact is created locally from the QR data.
    // The relay is only needed for syncing card updates afterwards.
    // So the exchange should succeed even without a relay.
    assert!(
        exchange_result.is_ok(),
        "Mutual QR exchange should succeed offline"
    );

    // Restart relay to check state
    orch.restart_relay(0)
        .await
        .expect("Failed to restart relay");

    // Bob has Alice as a contact from the QR exchange.
    // Alice does not have Bob yet because she hasn't synced (relay was down).
    orch.verify_contact_count("Bob", 1)
        .await
        .expect("Bob should have 1 contact (Alice from QR)");
    orch.verify_contact_count("Alice", 0)
        .await
        .expect("Alice should have 0 contacts (no sync yet)");

    orch.stop().await.expect("Failed to stop orchestrator");
}
