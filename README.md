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
| macOS remote (iOS) | `macos-remote.sh status` | SSH to macOS machine |
| Xcode (iOS) | Remote: `xcodebuild -version` | Mac App Store |

**Android Setup:**
```bash
# Verify environment
export ANDROID_HOME=/path/to/Android/Sdk
export ANDROID_SDK_ROOT=$ANDROID_HOME

# List available emulators
$ANDROID_HOME/emulator/emulator -list-avds

# Start emulator
$ANDROID_HOME/emulator/emulator -avd Pixel_7 &

# Verify Maestro can see device
maestro --device list
```

**iOS Setup (via macOS remote):**
```bash
# Check macOS connectivity
dev-tools/scripts/macos-remote.sh status

# List available simulators
dev-tools/scripts/macos-remote.sh simulator-list

# Boot simulator
dev-tools/scripts/macos-remote.sh simulator-boot "iPhone 15"

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

```bash
# List all E2E tests
just e2e

# Run specific test
just e2e-run test_cli_to_cli_exchange
just e2e-run test_multi_device_cli_linking

# Run all tests
just e2e-run all

# Run with verbose output
RUST_LOG=debug cargo test -p vauchi-e2e-tests -- --ignored --nocapture
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

### Relay not starting
```bash
# Check if port is in use
lsof -i :18080

# Kill stale processes
pkill -f vauchi-relay
```

### Android emulator not detected
```bash
# Verify ADB sees device
adb devices

# Restart ADB server
adb kill-server && adb start-server
```

### iOS Simulator not connecting
```bash
# Check macOS connectivity
dev-tools/scripts/macos-remote.sh status

# Verify simulators
dev-tools/scripts/macos-remote.sh run "xcrun simctl list devices booted"
```

### Maestro not finding app
```bash
# Ensure app is installed
maestro --device list

# Check app bundle ID
maestro studio  # Visual debugger
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
