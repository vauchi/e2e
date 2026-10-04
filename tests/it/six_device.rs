// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Arrange helpers shared by the six-device (A1-A3, B1-B3) certifications.

use vauchi_e2e_tests::prelude::*;

pub(crate) fn exchanged_contact_id(contacts: &[Contact], display_name: &str) -> String {
    contacts
        .iter()
        .find(|contact| contact.name == display_name && contact.id.is_some())
        .and_then(|contact| contact.id.clone())
        .unwrap_or_else(|| {
            panic!(
                "exchange should leave an addressable {display_name} contact; contacts={contacts:?}"
            )
        })
}

pub(crate) type SharedUser = std::sync::Arc<tokio::sync::RwLock<User>>;

/// Three linked devices per user through split OHTTP, A1<->B1 exchanged,
/// and the exchange synchronized to every linked device.
pub(crate) async fn exchanged_six_device_pair(orch: &mut Orchestrator) -> (SharedUser, SharedUser) {
    orch.add_user_split_ohttp("Alice", 3)
        .expect("Failed to add Alice through split OHTTP");
    orch.add_user_split_ohttp("Bob", 3)
        .expect("Failed to add Bob through split OHTTP");
    orch.create_all_identities()
        .await
        .expect("Failed to create identities");
    orch.link_all_devices()
        .await
        .expect("Failed to link all six devices");
    sync_rounds(orch, 2).await;

    let alice = orch.user("Alice").expect("Alice should exist");
    let bob = orch.user("Bob").expect("Bob should exist");
    {
        let alice = alice.read().await;
        let bob = bob.read().await;
        let alice_qr = alice
            .generate_qr_from_device(0)
            .await
            .expect("A1 should start exchange");
        let bob_qr = bob
            .generate_qr_from_device(0)
            .await
            .expect("B1 should start exchange");
        bob.complete_exchange_on_device(0, &alice_qr)
            .await
            .expect("B1 should complete exchange");
        alice
            .complete_exchange_on_device(0, &bob_qr)
            .await
            .expect("A1 should complete exchange");
    }
    sync_rounds(orch, 2).await;
    (alice, bob)
}

pub(crate) async fn sync_rounds(orch: &Orchestrator, rounds: usize) {
    for _ in 0..rounds {
        orch.sync_all().await.expect("sync round should succeed");
    }
}

/// Run sync rounds until `converged` reports true; false after `max_rounds`.
pub(crate) async fn sync_until<F, Fut>(
    orch: &Orchestrator,
    max_rounds: usize,
    mut converged: F,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..max_rounds {
        orch.sync_all().await.expect("sync round should succeed");
        if converged().await {
            return true;
        }
    }
    false
}

pub(crate) async fn contact_id_on_device(
    user: &SharedUser,
    device_index: usize,
    contact_name: &str,
) -> String {
    let contacts = user
        .read()
        .await
        .list_contacts_on_device(device_index)
        .await
        .expect("device should list contacts after exchange");
    exchanged_contact_id(&contacts, contact_name)
}
