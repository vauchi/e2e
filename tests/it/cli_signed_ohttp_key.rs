// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The CLI under e2e is anchored to its relay (#288 plan 6.7): the
//! orchestrator hands it the relay's test anchor, so it takes gateway keys
//! only from the relay's signed chain — as a production client will once
//! the production anchor exists. No key is compiled in any more.

use vauchi_e2e_tests::orchestrator::Orchestrator;

/// An anchored CLI refuses a signed key for a day it is not in; an
/// unanchored one would fetch the unsigned key and sync regardless. So the
/// refusal below shows the anchor reached the CLI and is enforced.
// @scenario: release_privacy_multidevice_certification.feature:Neither relay can decrypt or identify application users
#[tokio::test]
async fn the_cli_takes_keys_only_from_its_relays_signed_chain() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("start orchestrator");
    orch.add_user("Alice", 1).expect("add Alice");
    orch.add_user("Bob", 1).expect("add Bob");
    orch.create_all_identities()
        .await
        .expect("create identities");
    orch.exchange("Alice", "Bob")
        .await
        .expect("exchange with the relay's signed key");
    orch.sync_all()
        .await
        .expect("sync with the relay's signed key");

    orch.advance_relay_ohttp_windows(3)
        .await
        .expect("move the relay three days on");

    assert!(
        orch.sync_all().await.is_err(),
        "a CLI anchored to its relay must refuse a key for another day"
    );

    orch.stop().await.expect("stop orchestrator");
}
