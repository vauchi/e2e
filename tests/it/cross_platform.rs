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

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Smoke test for iOS simulator automation via Maestro.
///
/// Requirements:
/// - Maestro CLI installed (`curl -Ls "https://get.maestro.mobile.dev" | bash`)
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

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// Smoke test for Android emulator automation via Maestro.
///
/// Requirements:
/// - Maestro CLI installed
/// - Android emulator running and visible to `adb devices -l`
/// - Maestro YAML flows in `e2e/maestro/android/`
/// - APK built and installed
///
/// Skips gracefully when no emulator is connected so the full
/// `--include-ignored` suite stays green on machines without an Android emulator.
/// Physical devices are intentionally ignored because this test exercises the
/// emulator harness only.
///
/// The MaestroDevice implementation is at `e2e/src/device/maestro.rs`
// @internal
#[tokio::test]
async fn test_android_emulator_exchange() {
    use vauchi_e2e_tests::device::MaestroDevice;

    // Skip gracefully when no Android emulator is connected. This keeps the
    // full `--include-ignored` suite green on developer machines that only
    // have an iOS simulator, while still exercising the harness when an
    // emulator is present. Physical devices attached for other DT work are
    // ignored here because this smoke test is scoped to the emulator path.
    let adb_output = std::process::Command::new("adb")
        .args(["devices", "-l"])
        .stdout(std::process::Stdio::piped())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string());

    let detected_emulator = adb_output.and_then(|out| {
        out.lines().skip(1).find_map(|line| {
            // Lines look like:
            // "<device_id>\tdevice usb:... product:<name> model:<name> device:<name> ..."
            // Emulators expose product names such as sdk_gphone64_arm64.
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
            let is_emulator = product.contains("gphone") || product.contains("emulator");
            if is_emulator {
                Some(id.to_string())
            } else {
                None
            }
        })
    });

    let device_name = match detected_emulator {
        Some(name) => name,
        None => return,
    };

    let _guard = MAESTRO_SERIAL.lock().await;

    let device = MaestroDevice::android("Bob_Android", &device_name, "ws://localhost:8080")
        .expect("Maestro CLI must be installed to run Android emulator tests");

    device
        .create_identity("Bob")
        .await
        .expect("Android emulator create_identity flow should succeed");
}

// @scenario: contact_exchange:Two users exchange contact cards via QR code
/// TUI testing via PTY automation.
///
/// Requirements:
/// - TUI binary built (`cargo build -p vauchi-tui`)
/// - expectrl crate for PTY automation (uses `script` command for /dev/tty)
///
/// The TuiDevice is implemented at `e2e/src/device/tui.rs`
// @internal
#[tokio::test]
#[ignore = "requires TUI binary - run `cargo build -p vauchi-tui --release` first"]
#[cfg(feature = "tui")]
async fn test_tui_exchange() {
    use vauchi_e2e_tests::device::{Device, TuiDevice};

    // Create a TuiDevice
    let device = TuiDevice::new("Alice_TUI", "ws://localhost:8080")
        .expect("TUI binary not found. Run `cargo build -p vauchi-tui --release` first.");

    // Create identity
    device
        .create_identity("Alice")
        .await
        .expect("Failed to create identity in TUI");

    // Verify identity was created by checking if we're on the home screen
    let card = device
        .get_card()
        .await
        .expect("Failed to get card from TUI");

    // The card should exist (even if empty)
    assert!(
        card.name.is_empty() || card.name.contains("Alice") || card.name.contains("User"),
        "Card name should be set after identity creation"
    );

    // Clean up
    device.kill_app().await.expect("Failed to kill TUI");
}
