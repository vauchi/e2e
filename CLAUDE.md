<!-- SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# CLAUDE.md - vauchi-e2e-tests

> **Inherits**: See [CLAUDE.md](../CLAUDE.md) for project-wide rules.

End-to-end testing infrastructure for Vauchi.

## Component-Specific Rules

- Depends on `vauchi-core` and `vauchi-relay`
- Tests multi-user, multi-device scenarios
- Tests relay failover and offline recovery

## Commands

```bash
cargo test                                  # Run all e2e tests
cargo test five_user_exchange              # Run specific test
cargo test --release                        # Run tests in release mode
```

## Test Scenarios

- `five_user_exchange.rs` - Multi-user card exchange
- `multi_device_sync.rs` - Device synchronization
- `relay_failover.rs` - Relay redundancy testing
- `offline_catchup.rs` - Offline-to-online recovery
- `cross_platform.rs` - Cross-platform compatibility

## Local Development

Uses `.cargo/config.toml` to patch git dependencies to local paths.
Ensure `../core/vauchi-core` and `../relay` exist for local builds.
