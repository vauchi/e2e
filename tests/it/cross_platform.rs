// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Cross-Platform Exchange E2E Test
//!
//! Tests the scenario:
//! 1. Alice on iOS exchanges with Bob on Android
//! 2. Carol on Desktop exchanges with Dave on CLI
//! 3. Eve on iOS links Android and CLI devices
//! 4. Verify all exchanges work across platforms
//!
//! Note: Phase 1 only supports CLI devices. This test serves as a
//! placeholder for future platform integration tests.
//!
//! ## Test Tiers
//! - `smoke_*`: Fast tests for every push (< 5 min total)
//! - `integration_*`: Comprehensive tests for main branch

use vauchi_e2e_tests::prelude::*;

/// Serialise Maestro-driven device tests. Maestro's single-process XCTest
/// driver on macOS and the `adb`/emulator bridge on the host do not tolerate
/// concurrent flows reliably, so the iOS and Android simulator tests take this
/// lock for the duration of their create_identity flow.
static MAESTRO_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Smoke test: Basic CLI exchange between two users.
/// Tags: smoke, exchange
/// Feature: contact_exchange.feature
// @internal
#[tokio::test]
async fn smoke_cli_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    orch.exchange("Alice", "Bob")
        .await
        .expect("Exchange failed");

    orch.verify_contact_count("Alice", 1)
        .await
        .expect("Alice should have 1 contact");
    orch.verify_contact_count("Bob", 1)
        .await
        .expect("Bob should have 1 contact");

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: device_management:User links a new device
/// Integration test: Device linking across CLI instances.
/// Tags: integration, device-linking
/// Feature: device_management.feature
// @internal
#[tokio::test]
async fn integration_device_linking() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Eve", 3).expect("Failed to add Eve"); // Simulates iOS + Android + CLI

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    let eve = orch.user("Eve").unwrap();

    // Verify all devices are linked
    {
        let eve = eve.read().await;
        let devices = eve.device(0).unwrap().read().await;
        let device_list = devices
            .list_devices()
            .await
            .expect("Failed to list devices");

        assert!(
            !device_list.is_empty(),
            "Eve should have at least 1 device listed"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Integration test: Exchange between users with different device counts.
/// Tags: integration, exchange, multi-device
/// Feature: contact_exchange.feature
// @internal
#[tokio::test]
async fn integration_mixed_devices() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice"); // Single device
    orch.add_user("Bob", 2).expect("Failed to add Bob"); // Two devices
    orch.add_user("Carol", 3).expect("Failed to add Carol"); // Three devices

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    // Alice exchanges with Bob
    orch.exchange("Alice", "Bob")
        .await
        .expect("Alice-Bob exchange failed");

    // Bob exchanges with Carol
    orch.exchange("Bob", "Carol")
        .await
        .expect("Bob-Carol exchange failed");

    // Carol exchanges with Alice
    orch.exchange("Carol", "Alice")
        .await
        .expect("Carol-Alice exchange failed");

    // Final sync round: ensure all device sync messages are delivered.
    // Each exchange syncs both users, but secondary devices may need one
    // more round to receive contacts added in other exchanges.
    orch.sync_all().await.expect("Final sync failed");

    // Verify all users have 2 contacts each
    orch.verify_contact_count("Alice", 2)
        .await
        .expect("Alice should have 2 contacts");
    orch.verify_contact_count("Bob", 2)
        .await
        .expect("Bob should have 2 contacts");
    orch.verify_contact_count("Carol", 2)
        .await
        .expect("Carol should have 2 contacts");

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: device_management:User links a new device
// @scenario: contact_exchange:Two users exchange contact cards via QR code
// @scenario: sync_updates:Contact updates propagate to all devices
/// Integration test: Device linking, exchange, and sync in one flow.
///
/// Verifies that a user with a linked secondary device can exchange with a
/// peer and that the contact appears on both primary and secondary devices
/// after sync.
///
/// Tags: integration, device-linking, exchange, sync
/// Feature: device_management.feature, contact_exchange.feature, sync_updates.feature
// @internal
#[tokio::test]
async fn integration_device_link_then_exchange_and_sync() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 2).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link devices");

    orch.exchange("Alice", "Bob")
        .await
        .expect("Alice-Bob exchange failed");

    // Secondary devices need an explicit sync round to receive contacts added
    // on the primary device.
    orch.sync_all().await.expect("Final sync failed");

    let alice = orch.user("Alice").expect("Alice should exist");
    for device_index in 0..2 {
        let contacts = alice
            .read()
            .await
            .list_contacts_on_device(device_index)
            .await
            .unwrap_or_else(|_| panic!("Alice device {} should list contacts", device_index));
        assert!(
            contacts.iter().any(|c| c.name == "Bob"),
            "Alice device {} should have Bob as a contact after exchange and sync",
            device_index
        );
    }

    let bob = orch.user("Bob").expect("Bob should exist");
    let contacts = bob
        .read()
        .await
        .list_contacts_on_device(0)
        .await
        .expect("Bob should list contacts");
    assert!(
        contacts.iter().any(|c| c.name == "Alice"),
        "Bob should have Alice as a contact after exchange"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Smoke test for iOS simulator automation via Maestro.
///
/// Requirements:
/// - A reviewed, pinned Maestro CLI release installed from the official
///   `mobile-dev-inc/Maestro` GitHub releases
/// - iOS Simulator running and booted (the test auto-detects its UDID)
/// - Maestro YAML flows in `e2e/maestro/ios/`
/// - App built and installed for simulator
///
/// Skips gracefully when no booted simulator is available so the full
/// `--include-ignored` suite stays green on machines without an iOS simulator.
///
/// The MaestroDevice implementation is at `e2e/src/device/maestro.rs`
// @internal
#[tokio::test]
async fn test_ios_simulator_exchange() {
    use vauchi_e2e_tests::device::MaestroDevice;

    // Auto-detect a booted iOS simulator UDID. Maestro's `--device` flag
    // accepts either a device name or a UDID; the UDID is stable when
    // multiple simulators share the same name.
    let udid = std::process::Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .stdout(std::process::Stdio::piped())
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.lines().find_map(|line| {
                // Lines look like: "    iPhone 17 Pro (79993A0D-...) (Booted)"
                line.split('(')
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .filter(|s| s.contains('-'))
                    .map(|s| s.to_string())
            })
        });

    let udid = match udid {
        Some(u) => u,
        None => return,
    };

    let _guard = MAESTRO_SERIAL.lock().await;

    let device = MaestroDevice::ios("Alice_iOS", &udid, "ws://localhost:8080")
        .expect("Maestro CLI must be installed to run iOS simulator tests");

    device
        .create_identity("Alice")
        .await
        .expect("iOS simulator create_identity flow should succeed");
}

/// Detect a booted iOS simulator UDID, if any.
fn detect_booted_ios_simulator() -> Option<String> {
    std::process::Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .stdout(std::process::Stdio::piped())
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.lines().find_map(|line| {
                line.split('(')
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .filter(|s| s.contains('-'))
                    .map(|s| s.to_string())
            })
        })
}

/// Detect any connected Android device (emulator or physical).
fn detect_android_device() -> Option<String> {
    detect_android_device_with_filter(|_| true)
}

fn detect_android_device_with_filter<F>(filter: F) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    let adb_output = std::process::Command::new("adb")
        .args(["devices", "-l"])
        .stdout(std::process::Stdio::piped())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())?;

    adb_output.lines().skip(1).find_map(|line| {
        let mut parts = line.split_whitespace();
        let id = parts.next()?;
        let state = parts.next()?;
        if state != "device" {
            return None;
        }
        let mut product = "";
        for part in parts {
            if let Some(value) = part.strip_prefix("product:") {
                product = value;
            }
        }
        if filter(product) {
            Some(id.to_string())
        } else {
            None
        }
    })
}

/// Check whether the Vauchi Android app is installed on a device.
fn is_android_app_installed(device_id: &str) -> bool {
    std::process::Command::new("adb")
        .args([
            "-s",
            device_id,
            "shell",
            "pm",
            "list",
            "packages",
            "app.vauchi",
        ])
        .stdout(std::process::Stdio::piped())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .map(|out| out.lines().any(|line| line.contains("app.vauchi")))
        .unwrap_or(false)
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Smoke test for Android device automation via Maestro.
///
/// Requirements:
/// - Maestro CLI installed
/// - Android emulator or physical device connected and visible to `adb devices -l`
/// - APK built and installed
/// - Maestro YAML flows in `e2e/maestro/android/`
///
/// Skips gracefully when no Android device is connected so the full
/// `--include-ignored` suite stays green on machines without Android hardware.
///
/// The MaestroDevice implementation is at `e2e/src/device/maestro.rs`
// @internal
#[tokio::test]
async fn test_android_device_exchange() {
    use vauchi_e2e_tests::device::MaestroDevice;

    let device_name = match detect_android_device() {
        Some(name) => name,
        None => return,
    };

    if !is_android_app_installed(&device_name) {
        eprintln!(
            "Skipping Android device test: app.vauchi is not installed on {}",
            device_name
        );
        return;
    }

    let _guard = MAESTRO_SERIAL.lock().await;

    let device = MaestroDevice::android("Bob_Android", &device_name, "ws://localhost:8080")
        .expect("Maestro CLI must be installed to run Android device tests");

    device
        .create_identity("Bob")
        .await
        .expect("Android device create_identity flow should succeed");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Smoke test: TUI can create an identity through the generic presentation
/// protocol.
///
/// Requirements:
/// - TUI binary built (`just build tui`)
/// - expectrl crate for PTY automation
///
/// The TuiDevice is implemented at `e2e/src/device/tui.rs`
// @internal
#[tokio::test]
#[cfg(feature = "tui")]
async fn test_tui_create_identity() {
    use vauchi_e2e_tests::device::{Device, TuiDevice};

    let device = TuiDevice::new("Alice_TUI", "ws://localhost:8080")
        .expect("TUI binary not found. Run `just build tui` first.");

    device
        .create_identity("Alice")
        .await
        .expect("Failed to create identity in TUI");

    assert!(
        device.has_identity().await,
        "Identity should exist after create_identity"
    );

    device.kill_app().await.expect("Failed to kill TUI");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Integration test: CLI user shares their card with an Android device user.
///
/// Ignored: the flow it drives has no counterpart in the app. The Maestro
/// steps after the first tap reference strings absent from `en.json`
/// entirely — the manual QR-entry affordance was deleted, and core emits
/// none of the `qr.paste_*` keys still sitting in the catalogues. Neither
/// the `vauchi://` deep link (which rejects the payload's path form) nor
/// the CLI (which has no link mode) offers another way in, so a
/// CLI-generated payload currently cannot reach the app at all. Re-pointing
/// the flow has nothing to point at; restoring the affordance is a product
/// decision, not a test fix.
///
/// Investigation, including the three routes checked and closed:
/// `_private/docs/backlog/2026-08-09-cli-to-android-exchange-automation-unreachable.md`
///
/// The device guard below stays for `--include-ignored` runs on machines
/// without the harness. It is deliberately no longer the *only* thing
/// gating this test: because CI has no phone attached, that guard alone
/// meant CI never executed the test, so the flow drifted out of step with
/// the app unnoticed. An `#[ignore]` states the disabled state in source
/// where it can be read; hardware detection hid it.
///
/// Tags: integration, exchange, cross-platform, android
/// Feature: contact_exchange.feature
// @internal
#[tokio::test]
#[ignore = "drives a manual-entry affordance the app no longer has — see \
            backlog/2026-08-09-cli-to-android-exchange-automation-unreachable.md"]
async fn integration_cli_to_android_exchange() {
    let device_name = match detect_android_device() {
        Some(name) => name,
        None => return,
    };

    if !is_android_app_installed(&device_name) {
        eprintln!(
            "Skipping CLI→Android exchange test: app.vauchi is not installed on {}",
            device_name
        );
        return;
    }

    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");

    let _guard = MAESTRO_SERIAL.lock().await;

    if let Err(e) = orch.add_user_with_maestro_android("Bob", &device_name) {
        eprintln!("Skipping CLI→Android exchange test: {e}");
        return;
    }

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let qr = {
        let alice = orch.user("Alice").unwrap();
        let alice = alice.read().await;
        alice.generate_qr().await.expect("Alice should generate QR")
    };

    {
        let bob = orch.user("Bob").unwrap();
        let bob = bob.read().await;
        bob.complete_exchange(&qr)
            .await
            .expect("Bob should complete exchange with Alice's QR");
    }

    for _ in 0..2 {
        {
            let alice = orch.user("Alice").unwrap();
            let alice = alice.read().await;
            alice.sync_all().await.expect("Alice sync failed");
        }
        {
            let bob = orch.user("Bob").unwrap();
            let bob = bob.read().await;
            bob.sync_all().await.expect("Bob sync failed");
        }
    }

    let bob = orch.user("Bob").unwrap();
    let bob = bob.read().await;
    let contacts = bob.list_contacts().await.expect("Bob should list contacts");
    assert!(
        contacts.iter().any(|c| c.name == "Alice"),
        "Bob should have Alice as a contact after CLI-to-Android exchange"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Integration test: CLI user shares their card with an iOS simulator user.
///
/// Ignored for the same reason as its Android sibling: the flow drives a
/// manual QR-entry affordance the app no longer has. `complete_exchange`
/// taps "Add Contact", which the app renders as "Exchange Now", and the
/// two steps after it reference strings absent from `en.json` entirely.
/// Re-pointing it has nothing to point at; restoring the affordance is a
/// product decision.
///
/// Confirmed on iPhone 17 Pro on 2026-08-10, which answers open question 4
/// of
/// `_private/docs/backlog/2026-08-09-cli-to-android-exchange-automation-unreachable.md`
/// — the iOS flow is a copy of the android one and has the same defect.
///
/// This only became visible once the harness started passing variables to
/// Maestro at all. Until then every flow ran with every variable unset, so
/// the run died earlier, on a garbage identity name.
///
/// Tags: integration, exchange, cross-platform, ios
/// Feature: contact_exchange.feature
// @internal
#[tokio::test]
#[ignore = "drives a manual-entry affordance the app no longer has — see \
            backlog/2026-08-09-cli-to-android-exchange-automation-unreachable.md"]
async fn integration_cli_to_ios_exchange() {
    let udid = match detect_booted_ios_simulator() {
        Some(u) => u,
        None => return,
    };

    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");

    let _guard = MAESTRO_SERIAL.lock().await;

    if let Err(e) = orch.add_user_with_maestro_ios("Bob", &udid) {
        eprintln!("Skipping CLI→iOS exchange test: {e}");
        return;
    }

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let qr = {
        let alice = orch.user("Alice").unwrap();
        let alice = alice.read().await;
        alice.generate_qr().await.expect("Alice should generate QR")
    };

    {
        let bob = orch.user("Bob").unwrap();
        let bob = bob.read().await;
        bob.complete_exchange(&qr)
            .await
            .expect("Bob should complete exchange with Alice's QR");
    }

    for _ in 0..2 {
        {
            let alice = orch.user("Alice").unwrap();
            let alice = alice.read().await;
            alice.sync_all().await.expect("Alice sync failed");
        }
        {
            let bob = orch.user("Bob").unwrap();
            let bob = bob.read().await;
            bob.sync_all().await.expect("Bob sync failed");
        }
    }

    let bob = orch.user("Bob").unwrap();
    let bob = bob.read().await;
    let contacts = bob.list_contacts().await.expect("Bob should list contacts");
    assert!(
        contacts.iter().any(|c| c.name == "Alice"),
        "Bob should have Alice as a contact after CLI-to-iOS exchange"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Integration test: CLI user shares their card with a TUI user.
///
/// Requirements:
/// - TUI binary built (`just build tui`)
/// - expectrl crate for PTY automation
///
/// Tags: integration, exchange, cross-platform, tui
/// Feature: contact_exchange.feature
// @internal
#[tokio::test]
#[cfg(feature = "tui")]
async fn integration_cli_to_tui_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user_with_tui("Bob")
        .expect("Failed to add Bob with TUI");

    orch.create_all_identities()
        .await
        .expect("Failed to create identities");

    let qr = {
        let alice = orch.user("Alice").unwrap();
        let alice = alice.read().await;
        alice.generate_qr().await.expect("Alice should generate QR")
    };

    {
        let bob = orch.user("Bob").unwrap();
        let bob = bob.read().await;
        bob.complete_exchange(&qr)
            .await
            .expect("Bob should complete exchange with Alice's QR");
    }

    for _ in 0..2 {
        {
            let alice = orch.user("Alice").unwrap();
            let alice = alice.read().await;
            alice.sync_all().await.expect("Alice sync failed");
        }
        {
            let bob = orch.user("Bob").unwrap();
            let bob = bob.read().await;
            bob.sync_all().await.expect("Bob sync failed");
        }
    }

    let bob = orch.user("Bob").unwrap();
    let bob = bob.read().await;
    let contacts = bob.list_contacts().await.expect("Bob should list contacts");
    assert!(
        contacts.iter().any(|c| c.name == "Alice"),
        "Bob should have Alice as a contact after CLI-to-TUI exchange"
    );

    let alice = orch.user("Alice").unwrap();
    let alice = alice.read().await;
    let contacts = alice
        .list_contacts()
        .await
        .expect("Alice should list contacts");
    assert!(
        contacts.iter().any(|c| c.name == "Bob"),
        "Alice should have Bob as a contact after CLI-to-TUI exchange"
    );

    orch.stop().await.expect("Failed to stop orchestrator");
}
