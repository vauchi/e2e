<!-- SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

> [!WARNING]
> **Pre-Alpha Software** - This project is under heavy development and not ready for production use.
> APIs may change without notice. Use at your own risk.

# Vauchi E2E Testing Infrastructure

End-to-end testing framework for multi-user, multi-device, cross-platform scenarios.

## Prerequisites

### Phase 1: CLI Testing (Current)

| Requirement | Check Command | Install |
|-------------|---------------|---------|
| Rust toolchain | `rustc --version` | [rustup.rs](https://rustup.rs) |
| CLI binary | `just build-cli` | Built from source |
| Relay binary | `just build-relay` | Built from source |

```bash
# Build prerequisites
just build-cli
just build-relay

# Run CLI-based tests
just e2e-run test_cli_to_cli_exchange
```

### Phase 2: Mobile Testing (Maestro)

| Requirement | Check Command | Install |
|-------------|---------------|---------|
| Maestro CLI | `maestro --version` | `curl -Ls "https://get.maestro.mobile.dev" \| bash` |
| Android SDK | `echo $ANDROID_HOME` | [Android Studio](https://developer.android.com/studio) |
| Android emulator | `emulator -list-avds` | Android Studio AVD Manager |
| Xcode (iOS, macOS only) | `xcodebuild -version` | Mac App Store |
| iOS Simulator (macOS only) | `xcrun simctl list` | Xcode |

> **Quick check:** Run `just maestro-setup` to verify your entire Maestro
> environment in one step. It checks the CLI, iOS tooling, Android tooling,
> and available flow files.

**Android Setup:**
```bash
# Verify environment
export ANDROID_HOME=/path/to/Android/Sdk
export ANDROID_SDK_ROOT=$ANDROID_HOME
export PATH=$PATH:$ANDROID_HOME/emulator:$ANDROID_HOME/platform-tools

# List available emulators
$ANDROID_HOME/emulator/emulator -list-avds

# Start emulator
$ANDROID_HOME/emulator/emulator -avd Pixel_7 &

# Verify Maestro can see device (explicit platform)
maestro test --platform android e2e/maestro/android/create_identity.yaml
```

**iOS Setup (local macOS):**
```bash
# Boot a simulator
xcrun simctl boot "iPhone 15 Pro"

# Install the app (from ios/ repo)
cd ios && xcodebuild -scheme Vauchi \
  -destination 'platform=iOS Simulator,name=iPhone 15 Pro' \
  -configuration Debug build

# Verify Maestro can see simulator (explicit platform)
maestro test --platform ios e2e/maestro/ios/create_identity.yaml
```

**iOS Setup (via macOS remote):**
```bash
# Check macOS connectivity
dev-tools/scripts/macos-remote.sh status

# List available simulators
dev-tools/scripts/macos-remote.sh simulator-list

# Boot simulator
dev-tools/scripts/macos-remote.sh simulator-boot "iPhone 15 Pro"

# Sync project to macOS
dev-tools/scripts/macos-remote.sh sync
```

### Phase 3: Desktop Testing (Tauri + WebdriverIO)

| Requirement | Check Command | Install |
|-------------|---------------|---------|
| Tauri CLI | `cargo tauri --version` | `cargo install tauri-cli` |
| Node.js | `node --version` | [nodejs.org](https://nodejs.org) |
| WebdriverIO | `npx wdio --version` | `npm install @wdio/cli` |
| Desktop build | `cargo tauri build` | Built from source |

**Note:** Tauri WebDriver testing is only supported on Linux/Windows (not macOS).

### Phase 4: TUI Testing (expectrl)

| Requirement | Check Command | Install |
|-------------|---------------|---------|
| TUI binary | `just build` | Built from source |
| expectrl | Cargo dependency | Added to Cargo.toml |

## Environment Variables

```bash
# Android
export ANDROID_HOME=/home/$USER/Android/Sdk
export ANDROID_SDK_ROOT=$ANDROID_HOME
export PATH=$PATH:$ANDROID_HOME/emulator:$ANDROID_HOME/platform-tools

# Maestro
export PATH=$PATH:$HOME/.maestro/bin
export MAESTRO_CLI_NO_ANALYTICS=true  # Optional: disable analytics

# macOS Remote (for iOS)
export MACOS_VM_IP=192.168.x.x
export MACOS_VM_USER=username
export PROJECT_PATH=/Volumes/Workspace/vauchi
```

## Running Tests

### CLI E2E (Rust harness)

```bash
# List all E2E tests
just e2e

# Run specific test
just e2e-run test_cli_to_cli_exchange
just e2e-run test_multi_device_cli_linking

# Run all tests
just e2e-run all

# Run with verbose output
RUST_LOG=debug just e2e-run all
```

### Mobile E2E (Maestro)

```bash
# Check environment first
just maestro-setup

# Run a single iOS flow
just e2e-ios create_identity
just e2e-ios generate_qr

# Run a single Android flow
just e2e-android create_identity
just e2e-android complete_exchange

# Run all flows on both platforms
just e2e-maestro

# Run all flows on one platform
just e2e-maestro ios
just e2e-maestro android

# Run Rust-orchestrated mobile tests (requires booted devices)
just e2e-mobile
```

### All Platforms

```bash
# Full suite (CLI + Desktop + Mobile)
just e2e-all
```

## Test Status

| Test | Phase | Status | Notes |
|------|-------|--------|-------|
| CLI-to-CLI exchange | 1 | Working | Basic contact exchange |
| Multi-device linking | 1 | Working | Device pairing |
| Contact sync across devices | 1 | Failing | Multi-device sync issue |
| Five user exchange | 1 | Failing | Multi-device sync issue |
| Visibility labels | 1 | New | Label CRUD, field visibility per label |
| Recovery flow | 1 | New | Social recovery with vouchers |
| Per-contact visibility | 1 | New | Hide/show fields per contact |
| Backup & restore | 1 | New | Identity export/import |
| iOS Simulator | 2 | Placeholder | Maestro flows for device linking, labels |
| Android Emulator | 2 | Placeholder | Requires Maestro flows |
| Desktop (Tauri) | 3 | Placeholder | Requires WebdriverIO |
| TUI | 4 | Placeholder | Requires expectrl |

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                     E2E Test Orchestrator (Rust)                    │
├─────────────────────────────────────────────────────────────────────┤
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐          │
│  │   Relay A    │    │   Relay B    │    │  Test Clock  │          │
│  │  :18080      │    │  :18081      │    │  (simulated) │          │
│  └──────────────┘    └──────────────┘    └──────────────┘          │
│         │                   │                                       │
│  ┌──────┴───────────────────┴──────────────────────────────┐       │
│  │              Device Abstraction Layer                    │       │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐        │       │
│  │  │CliDevice│ │Maestro  │ │ Tauri   │ │   TUI   │        │       │
│  │  │  (CLI)  │ │ (Mobile)│ │(Desktop)│ │(Terminal│        │       │
│  │  └─────────┘ └─────────┘ └─────────┘ └─────────┘        │       │
│  └──────────────────────────────────────────────────────────┘       │
└─────────────────────────────────────────────────────────────────────┘
```

## Troubleshooting

### iOS XCTest driver timeout

**Symptom:** Maestro hangs for 30-60 seconds then fails with
`XCTest driver connection timed out` or `Failed to connect to XCTestRunner`.

**Cause:** When both an iOS simulator and Android emulator are running,
Maestro's auto-detection may attempt to connect to the Android device first
using the XCTest driver, which will never succeed.

**Fix:** Always pass `--platform ios` when targeting iOS. The `just` recipes
and the Rust `MaestroDevice` do this automatically:

```bash
# WRONG (may try Android driver first)
maestro test e2e/maestro/ios/create_identity.yaml

# CORRECT (explicit platform selection)
maestro test --platform ios e2e/maestro/ios/create_identity.yaml

# Best: use just recipes (--platform is automatic)
just e2e-ios create_identity
```

If the problem persists after using `--platform`:

```bash
# Verify a simulator is booted
xcrun simctl list devices booted

# Boot one if none are running
xcrun simctl boot "iPhone 15 Pro"

# Check Maestro can see it
maestro --platform ios hierarchy
```

### Android emulator not targeted correctly

**Symptom:** Maestro runs the flow on the iOS simulator instead of the
Android emulator, or fails with `No Android devices found`.

**Cause:** Without `--platform android`, Maestro may prefer the iOS
simulator when both are running.

**Fix:** Always pass `--platform android` when targeting Android:

```bash
# WRONG (may target iOS simulator)
maestro test e2e/maestro/android/create_identity.yaml

# CORRECT (explicit platform selection)
maestro test --platform android e2e/maestro/android/create_identity.yaml

# Best: use just recipes (--platform is automatic)
just e2e-android create_identity
```

If the problem persists:

```bash
# Verify ADB sees the emulator
adb devices

# Restart ADB if needed
adb kill-server && adb start-server

# Check ANDROID_HOME is set
echo $ANDROID_HOME
```

### Relay not starting

```bash
# Check if port is in use
lsof -i :18080

# Kill stale processes
pkill -f vauchi-relay
```

### Android emulator not detected by ADB

```bash
# Verify ADB sees device
adb devices

# Restart ADB server
adb kill-server && adb start-server

# Check emulator is running
emulator -list-avds
```

### iOS Simulator not connecting (remote macOS)

```bash
# Check macOS connectivity
dev-tools/scripts/macos-remote.sh status

# Verify simulators
dev-tools/scripts/macos-remote.sh run "xcrun simctl list devices booted"
```

### Maestro not finding app

```bash
# Ensure app is installed on the correct platform
maestro --platform ios hierarchy    # iOS
maestro --platform android hierarchy  # Android

# Check app bundle IDs
# iOS: app.vauchi.ios
# Android: com.vauchi

# Use Maestro Studio for visual debugging
maestro studio
```

### Environment check fails

```bash
# Run the full environment diagnostic
just maestro-setup

# This checks: Maestro CLI, Xcode tools, Android SDK, ADB,
# booted simulators/emulators, and available flow files.
```

## Adding New Tests

1. Create test file in `tests/`
2. Use `Orchestrator` or `Scenario` DSL
3. Mark with `#[ignore]` until infrastructure ready
4. Add to test list in this README

Example:
```rust
#[tokio::test]
#[ignore = "requires Maestro and Android emulator"]
async fn test_android_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.unwrap();
    // ... test implementation
    orch.stop().await.unwrap();
}
```

## See Also

- [Planning doc](../docs/planning/todo/09-e2e-testing-infrastructure.md)
- [Maestro docs](https://maestro.mobile.dev)
- [Tauri testing](https://v2.tauri.app/develop/tests/webdriver/)
