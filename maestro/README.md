<!-- SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Maestro Flows for Mobile E2E Testing

This directory contains Maestro YAML flows for automated mobile testing.

## Flow Maintenance Rules (learned 2026-07-18 flow-rot sweep)

- **Address elements by Core's accessibility label, never by a test id.**
  ADR-066 left no stable id to target. Core mints interaction and binding
  ids per surface revision (`surface.{revision}.interaction.{n}`), so they
  change whenever the surface does; and the shell cannot mint a semantic
  id like `home.settings` of its own, because naming a node "settings"
  means knowing domain vocabulary ADR-066 denies it. The label is the only
  stable, shell-visible handle. The ids that used to work
  (`tab_my_info`, `home.settings`, `card_view`, …) predate ADR-066, when
  the shell owned its screens; on 2026-08-12 none of the twelve still
  existed in the app, whose only test tags are `error.retry`,
  `recovery.restore_backup` and `recovery.start_fresh`.
  Caveat: this couples flows to copy, so it holds only while the app is
  unlocalized. Localizing the shell requires Core to supply stable
  revision-independent identifiers first — a protocol addition, not a
  flow change.
- **Navigation (Android, ADR-066)**: the app opens on the card surface and
  destinations live behind the context bar's navigation role button —
  `tapOn: "More"` then the destination (`My Card`, `Contacts`,
  `Exchange`, `Groups`, `Settings`, `Recovery`, `Devices`, `Backup`,
  `Privacy`, `Support`). Surface-specific actions such as `Add Entry` are
  under the secondary role button: `tapOn: "Actions"` first. Destructive
  actions live under **Settings → Advanced** (Danger Zone: Emergency
  Wipe, Wipe All Data); **Delete Identity** lives under **More → Privacy**
  (GDPR screen). The iOS flows still use `tab.*` ids and have not been
  re-pointed — the iOS gate has never run against a connected device, so
  their state is unverified rather than known-good.
- **Never use `hideKeyboard` on iOS** — it fires a phantom action
  equivalent to a second submit (onboarding skipped `groups_setup`).
  Android tolerates it. The action footer is tappable with the keyboard
  up; just delete the step.
- **Scroll before tapping below-fold rows**: after opening Settings,
  `waitForAnimationToEnd`, then `scrollUntilVisible` to the target row
  before `tapOn` (see `add_decoy_contact.yaml` for the pattern).
- **Physical iOS devices** need `maestro test --device <udid>
  --apple-team-id L2853TNSJ4` and an **unlocked** phone (no remote-unlock
  API exists). First run also needs
  `MAESTRO_DRIVER_STARTUP_TIMEOUT=240000` while the xctestrunner driver
  installs. `just device-gate ios` handles all of this.
- **Android physical**: the device gate wakes/unlocks per flow
  (`PIN=123456` default). If maestro reports
  `UNAVAILABLE / Connection refused: ...:7001`, the driver died — restart
  it: `adb shell am instrument -w
  dev.mobile.maestro.test/androidx.test.runner.AndroidJUnitRunner &` and
  `adb forward tcp:7001 tcp:7001`.

## Setup

1. Select a specific Maestro CLI version from the
   [official releases](https://github.com/mobile-dev-inc/Maestro/releases).
   Verify the signed release, inspect the downloaded archive before installing
   it, and confirm the reviewed version with `maestro --version`.

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

```text
maestro/
├── ios/                    # iOS-specific flows
│   ├── create_identity.yaml
│   ├── generate_qr.yaml
│   ├── complete_exchange.yaml
│   ├── sync.yaml
│   ├── list_contacts.yaml
│   ├── add_field.yaml
│   ├── get_card.yaml
│   ├── link_device.yaml
│   ├── visibility_labels.yaml
│   ├── setup_app_password.yaml      # Resistance features
│   ├── setup_duress_pin.yaml
│   ├── add_decoy_contact.yaml
│   ├── delete_decoy_contact.yaml
│   ├── duress_unlock.yaml
│   ├── hide_contact.yaml
│   ├── configure_emergency_broadcast.yaml
│   ├── send_emergency_broadcast.yaml
│   ├── identity_purge.yaml            # Identity purge (schedule + cancel)
│   └── emergency_shred.yaml           # Panic shred (destroys identity)
├── android/                # Android-specific flows (same set)
│   └── ...
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

> **Important:** Always pass `--platform` to avoid XCTest driver timeouts
> on iOS and incorrect device targeting on Android. The `just` recipes
> handle this automatically.

```bash
# Preferred: use just recipes (handles --platform automatically)
just e2e-ios create_identity
just e2e-android create_identity
just e2e-maestro              # Run all flows on both platforms

# Manual invocation (always include --platform)
maestro test --platform ios ios/create_identity.yaml
maestro test --platform android android/create_identity.yaml

# Run with variables
NAME=Alice maestro test --platform ios ios/create_identity.yaml

# Run on specific device
maestro test --platform ios --device "iPhone 15 Pro" ios/create_identity.yaml
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
| `link_device()` | `link_device.yaml` |
| `visibility_labels()` | `visibility_labels.yaml` |

### Resistance Feature Flows

These flows test security/resistance features and must be run in order
(1-5 form a chain: password > duress > decoys > unlock):

| # | Flow | Env Vars |
|---|------|----------|
| 1 | `setup_app_password.yaml` | `APP_PASSWORD` |
| 2 | `setup_duress_pin.yaml` | `DURESS_PIN` |
| 3 | `add_decoy_contact.yaml` | `DECOY_NAME` |
| 4 | `delete_decoy_contact.yaml` | `DECOY_NAME` |
| 5 | `duress_unlock.yaml` | `DURESS_PIN`, `DECOY_NAME`, `REAL_CONTACT_NAME` |
| 6 | `hide_contact.yaml` | `CONTACT_NAME` |
| 7 | `configure_emergency_broadcast.yaml` | `CONTACT_NAME`, `ALERT_MESSAGE` |
| 8 | `send_emergency_broadcast.yaml` | — |
| 9 | `identity_purge.yaml` | — |
| 10 | `emergency_shred.yaml` | — (WARNING: destroys identity, run last) |

## QR Code Extraction

For `generate_qr`, the flow should either:

1. Take a screenshot and save to a known path
2. Copy QR data to clipboard
3. Output QR data to stdout in a parseable format

The E2E framework will then extract the QR data for use in exchange tests.
