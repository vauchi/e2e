<!-- SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Shared feature-driver pilot

The Link scenario in the features repository's `link_exchange.feature`
runs through the same Cucumber step definitions with two drivers:

- Contract: AppEngine receives generic events with ids resolved from its
  emitted presentation. Only boot requests a snapshot; subsequent state
  follows emitted batches. It uses temporary storage, a controlled clock and
  a loopback escrow adapter backed by the production escrow store.
- CLI: the existing subprocess adapter runs the real CLI and local
  relay/OHTTP services. Assertions use CLI contact-list output.

Both paths require Alice to observe only Bob and Bob to observe only Alice
in their contact lists. A separate test blocks all escrow requests and
requires an exchange error and empty contact lists on both sides.
Another drops presentation updates after boot and requires failure before
any escrow request. This prevents an extra snapshot from hiding missing
renderer commands. Scheduled wakeups are consumed once and rearmed only
when Core emits a new scheduling command.
Missing specifications, unbound steps, parser failures and hook failures
fail the pilot instead of producing skipped coverage.

From the workspace root, with the existing E2E binaries built:

```sh
VAUCHI_E2E_NO_DEVICES=1 just exec-in-repo e2e \
  'cargo test --test it integration_shared_driver'
VAUCHI_E2E_NO_DEVICES=1 just exec-in-repo e2e \
  'cargo test --test it integration_cli_to_cli_link_exchange'
```

The default feature source is the sibling features checkout. CI fetches
features at `main`, logs the checked-out commit and sets
`VAUCHI_FEATURES_DIR`. Set `VAUCHI_FEATURES_REF` to test another canonical
feature revision in CI. Land the new feature before the E2E consumer.

The contract driver observes platform commands such as scheduled wakeups
and sharing. Link escrow I/O currently runs inside Core, so it is observed
at the HTTP adapter instead of being presented as a shell-side operation.
Its direct loopback transport is test-only. The CLI path retains OHTTP.

This pilot verifies Link exchange, not QR proximity, physical hardware,
GTK rendering or device input. Renderer fixtures and message-shape
compatibility tests remain separate. Extend the interface when another
scenario demonstrates a shared operation; do not add speculative Device
methods or copy exchange policy into a driver.

The minimal model covers one primary surface and explicit overlay
dismissal. It currently retains overlays until dismissal; a scenario that
replaces a surface while an overlay is open needs a model extension and
its own regression test before reuse.
