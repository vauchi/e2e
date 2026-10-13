// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Two CLI users exchange over a link (private#670): Alice runs
//! `vauchi exchange link` and keeps it waiting, Bob runs
//! `vauchi exchange link <Alice's link>`; the relay escrow pairs them.

use vauchi_e2e_tests::orchestrator::Orchestrator;

// @scenario: contact_exchange:Two users exchange contact cards via QR code
// @internal
#[tokio::test]
async fn integration_cli_to_cli_link_exchange() {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("start");
    orch.add_user("Alice", 1).expect("add Alice");
    orch.add_user("Bob", 1).expect("add Bob");
    orch.create_all_identities().await.expect("identities");

    orch.exchange_link("Alice", "Bob")
        .await
        .expect("Alice (sharing) and Bob (accepting) converge over a link");

    for (owner, peer) in [("Alice", "Bob"), ("Bob", "Alice")] {
        let user = orch.user(owner).expect("user");
        let user = user.read().await;
        let contacts = user.list_contacts().await.expect("contacts");
        assert!(
            contacts.iter().any(|c| c.name == peer),
            "{owner} has {peer} after the link exchange: {contacts:?}"
        );
    }
    orch.stop().await.expect("stop");
}
