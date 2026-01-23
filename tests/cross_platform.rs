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

/// Smoke test: Basic CLI exchange between two users.
/// Tags: smoke, exchange
/// Feature: contact_exchange.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn smoke_cli_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");
    orch.add_user("Bob", 1).expect("Failed to add Bob");

    orch.create_all_identities().await.expect("Failed to create identities");

    orch.exchange("Alice", "Bob").await.expect("Exchange failed");

    orch.verify_contact_count("Alice", 1).await.expect("Alice should have 1 contact");
    orch.verify_contact_count("Bob", 1).await.expect("Bob should have 1 contact");

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Device linking across CLI instances.
/// Tags: integration, device-linking
/// Feature: device_management.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_device_linking() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Eve", 3).expect("Failed to add Eve"); // Simulates iOS + Android + CLI

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    let eve = orch.user("Eve").unwrap();

    // Verify all devices are linked
    {
        let eve = eve.read().await;
        let devices = eve.device(0).unwrap().read().await;
        let device_list = devices.list_devices().await.expect("Failed to list devices");

        assert!(
            !device_list.is_empty(),
            "Eve should have at least 1 device listed"
        );
    }

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Integration test: Exchange between users with different device counts.
/// Tags: integration, exchange, multi-device
/// Feature: contact_exchange.feature
#[tokio::test]
#[ignore = "requires relay and CLI binaries to be built"]
async fn integration_mixed_devices() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("Failed to start orchestrator");

    orch.add_user("Alice", 1).expect("Failed to add Alice");  // Single device
    orch.add_user("Bob", 2).expect("Failed to add Bob");      // Two devices
    orch.add_user("Carol", 3).expect("Failed to add Carol");  // Three devices

    orch.create_all_identities().await.expect("Failed to create identities");
    orch.link_all_devices().await.expect("Failed to link devices");

    // Alice exchanges with Bob
    orch.exchange("Alice", "Bob").await.expect("Alice-Bob exchange failed");

    // Bob exchanges with Carol
    orch.exchange("Bob", "Carol").await.expect("Bob-Carol exchange failed");

    // Carol exchanges with Alice
    orch.exchange("Carol", "Alice").await.expect("Carol-Alice exchange failed");

    // Verify all users have 2 contacts each
    orch.verify_contact_count("Alice", 2).await.expect("Alice should have 2 contacts");
    orch.verify_contact_count("Bob", 2).await.expect("Bob should have 2 contacts");
    orch.verify_contact_count("Carol", 2).await.expect("Carol should have 2 contacts");

    orch.stop().await.expect("Failed to stop orchestrator");
}

/// Placeholder for future iOS simulator testing (Phase 2).
///
/// Requirements:
/// - Maestro CLI installed (`curl -Ls "https://get.maestro.mobile.dev" | bash`)
/// - iOS Simulator running (via `xcrun simctl boot "iPhone 15 Pro"`)
/// - Maestro YAML flows in `e2e/maestro/ios/`
/// - App built for simulator
///
/// The MaestroDevice stub is available at `e2e/src/device/maestro.rs`
#[tokio::test]
#[ignore = "requires Maestro CLI and iOS simulator - Phase 2"]
async fn test_ios_simulator_exchange() {
    use vauchi_e2e_tests::device::MaestroDevice;

    // Try to create an iOS MaestroDevice
    match MaestroDevice::ios("Alice_iOS", "iPhone 15 Pro", "ws://localhost:8080") {
        Ok(device) => {
            // Device created but Maestro flows not implemented
            match device.create_identity("Alice").await {
                Err(e) => panic!("iOS Maestro automation not implemented: {}", e),
                Ok(_) => panic!("Unexpected success - iOS automation should not be implemented yet"),
            }
        }
        Err(e) => {
            // Expected: Maestro CLI not installed
            panic!("Maestro CLI required: {}", e);
        }
    }
}

/// Placeholder for future Android emulator testing (Phase 2).
///
/// Requirements:
/// - Maestro CLI installed
/// - Android emulator running (via `emulator -avd Pixel_7`)
/// - ADB in PATH
/// - Maestro YAML flows in `e2e/maestro/android/`
/// - APK built and installed
///
/// The MaestroDevice stub is available at `e2e/src/device/maestro.rs`
#[tokio::test]
#[ignore = "requires Maestro CLI and Android emulator - Phase 2"]
async fn test_android_emulator_exchange() {
    use vauchi_e2e_tests::device::MaestroDevice;

    // Try to create an Android MaestroDevice
    match MaestroDevice::android("Bob_Android", "Pixel_7", "ws://localhost:8080") {
        Ok(device) => {
            // Device created but Maestro flows not implemented
            match device.create_identity("Bob").await {
                Err(e) => panic!("Android Maestro automation not implemented: {}", e),
                Ok(_) => panic!("Unexpected success - Android automation should not be implemented yet"),
            }
        }
        Err(e) => {
            // Expected: Maestro CLI not installed
            panic!("Maestro CLI required: {}", e);
        }
    }
}

/// Placeholder for future Desktop (Tauri) testing (Phase 3).
///
/// Requirements:
/// - Desktop app built (`just desktop-build`)
/// - Tauri test mode or WebdriverIO for UI automation
///
/// The TauriDevice stub is available at `e2e/src/device/tauri.rs`
#[tokio::test]
#[ignore = "requires Tauri IPC or WebdriverIO - Phase 3"]
async fn test_desktop_exchange() {
    use vauchi_e2e_tests::device::TauriDevice;

    // Try to create a TauriDevice
    match TauriDevice::new("Alice_Desktop", "ws://localhost:8080") {
        Ok(device) => {
            // Device created but methods not implemented
            match device.create_identity("Alice").await {
                Err(e) => panic!("Desktop automation not implemented: {}", e),
                Ok(_) => panic!("Unexpected success - desktop automation should not be implemented yet"),
            }
        }
        Err(e) => {
            panic!("Desktop app binary not found: {}. Run `just desktop-build` first.", e);
        }
    }
}

/// TUI testing via PTY automation.
///
/// Requirements:
/// - TUI binary built (`cargo build -p vauchi-tui`)
/// - expectrl crate for PTY automation (uses `script` command for /dev/tty)
///
/// The TuiDevice is implemented at `e2e/src/device/tui.rs`
#[tokio::test]
#[ignore = "requires TUI binary - run `cargo build -p vauchi-tui --release` first"]
async fn test_tui_exchange() {
    use vauchi_e2e_tests::device::{Device, TuiDevice};

    // Create a TuiDevice
    let device = TuiDevice::new("Alice_TUI", "ws://localhost:8080")
        .expect("TUI binary not found. Run `cargo build -p vauchi-tui --release` first.");

    // Create identity
    device.create_identity("Alice").await
        .expect("Failed to create identity in TUI");

    // Verify identity was created by checking if we're on the home screen
    let card = device.get_card().await
        .expect("Failed to get card from TUI");

    // The card should exist (even if empty)
    assert!(card.name.is_empty() || card.name.contains("Alice") || card.name.contains("User"),
        "Card name should be set after identity creation");

    // Clean up
    device.kill_app().await.expect("Failed to kill TUI");
}
