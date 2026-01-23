# Maestro Flows for Mobile E2E Testing

This directory contains Maestro YAML flows for automated mobile testing.

## Setup

1. Install Maestro CLI:
   ```bash
   curl -Ls "https://get.maestro.mobile.dev" | bash
   ```

2. For iOS:
   ```bash
   # Boot a simulator
   xcrun simctl boot "iPhone 15 Pro"

   # Build and install the app
   cd ios && xcodebuild -scheme Vauchi -destination 'platform=iOS Simulator,name=iPhone 15 Pro'
   ```

3. For Android:
   ```bash
   # Start an emulator
   emulator -avd Pixel_7

   # Install the APK
   adb install android/app/build/outputs/apk/debug/app-debug.apk
   ```

## Directory Structure

```
maestro/
├── ios/                    # iOS-specific flows
│   ├── create_identity.yaml
│   ├── generate_qr.yaml
│   ├── complete_exchange.yaml
│   ├── sync.yaml
│   └── list_contacts.yaml
├── android/                # Android-specific flows
│   ├── create_identity.yaml
│   ├── generate_qr.yaml
│   ├── complete_exchange.yaml
│   ├── sync.yaml
│   └── list_contacts.yaml
└── README.md               # This file
```

## Flow Template

Each flow should:
1. Navigate to the relevant screen
2. Perform the action
3. Verify success
4. Output any required data (e.g., QR codes)

Example `create_identity.yaml`:
```yaml
appId: app.vauchi.mobile
---
- launchApp:
    clearState: true
- tapOn: "Create Identity"
- inputText: ${NAME}
- tapOn: "Continue"
- assertVisible: "Identity created"
```

## Running Flows

```bash
# Run a single flow
maestro test ios/create_identity.yaml

# Run with variables
NAME=Alice maestro test ios/create_identity.yaml

# Run on specific device
maestro test --device "iPhone 15 Pro" ios/create_identity.yaml
```

## Integration with E2E Tests

The `MaestroDevice` in `e2e/src/device/maestro.rs` executes these flows
programmatically. Each Device trait method maps to a corresponding flow:

| Method | Flow |
|--------|------|
| `create_identity(name)` | `create_identity.yaml` |
| `generate_qr()` | `generate_qr.yaml` |
| `complete_exchange(qr)` | `complete_exchange.yaml` |
| `sync()` | `sync.yaml` |
| `list_contacts()` | `list_contacts.yaml` |
| `add_field(...)` | `add_field.yaml` |
| `get_card()` | `get_card.yaml` |

## QR Code Extraction

For `generate_qr`, the flow should either:
1. Take a screenshot and save to a known path
2. Copy QR data to clipboard
3. Output QR data to stdout in a parseable format

The E2E framework will then extract the QR data for use in exchange tests.
