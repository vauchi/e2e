// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The previous store release's CLI and the current one, through the real
//! relay and OHTTP relay (vauchi/private#1111). Store review keeps both in
//! users' hands for weeks; the support window is the previous minor
//! release, starting at the first store release. Until that release,
//! `PREVIOUS_CLI_SHA=none` and the current CLI stands in for both sides.

use vauchi_e2e_tests::prelude::*;

const PREVIOUS_EMAIL: &str = "previous.release@example.com";
const CURRENT_EMAIL: &str = "current.release@example.com";

async fn started() -> Orchestrator {
    let mut orch = Orchestrator::new();
    orch.start().await.expect("start orchestrator");
    orch
}

async fn emails_seen_by(orch: &Orchestrator, viewer: &str, contact: &str) -> Vec<String> {
    let viewer = orch.user(viewer).expect("viewer");
    let card = viewer
        .read()
        .await
        .get_contact_card(contact)
        .await
        .expect("read the contact's card")
        .unwrap_or_else(|| panic!("{contact} is not a contact"));
    card.fields
        .into_iter()
        .filter(|field| field.field_type == "email")
        .map(|field| field.value)
        .collect()
}

async fn share_email(orch: &Orchestrator, owner: &str, contact: &str, email: &str) {
    let user = orch.user(owner).expect("owner");
    let user = user.read().await;
    user.add_field("email", "Email", email)
        .await
        .expect("add the email");
    user.device(0)
        .expect("device")
        .read()
        .await
        .unhide_field_to_contact(contact, "Email")
        .await
        .expect("show the email to the contact");
    user.sync_all().await.expect("send the update");
}

// @internal
#[tokio::test]
async fn integration_the_previous_release_and_the_current_exchange_and_see_each_others_updates() {
    let mut orch = started().await;
    orch.add_user_on_previous_release("Previous")
        .expect("add the previous release");
    orch.add_user("Current", 1)
        .expect("add the current release");
    orch.create_all_identities().await.expect("identities");
    orch.exchange("Previous", "Current")
        .await
        .expect("exchange across releases");

    share_email(&orch, "Previous", "Current", PREVIOUS_EMAIL).await;
    share_email(&orch, "Current", "Previous", CURRENT_EMAIL).await;
    orch.sync_all().await.expect("sync both");

    assert_eq!(
        emails_seen_by(&orch, "Current", "Previous").await,
        [PREVIOUS_EMAIL]
    );
    assert_eq!(
        emails_seen_by(&orch, "Previous", "Current").await,
        [CURRENT_EMAIL]
    );
    orch.stop().await.expect("stop orchestrator");
}

// @internal
#[tokio::test]
async fn integration_a_link_from_the_previous_release_opens_on_the_current() {
    let mut orch = started().await;
    orch.add_user_on_previous_release("Previous")
        .expect("add the previous release");
    orch.add_user("Current", 1)
        .expect("add the current release");
    orch.create_all_identities().await.expect("identities");

    orch.exchange_link("Previous", "Current")
        .await
        .expect("link from the previous release");

    orch.verify_contact_count("Previous", 1)
        .await
        .expect("Previous has Current");
    orch.verify_contact_count("Current", 1)
        .await
        .expect("Current has Previous");
    orch.stop().await.expect("stop orchestrator");
}

// @internal
#[tokio::test]
async fn integration_a_link_from_the_current_release_opens_on_the_previous() {
    let mut orch = started().await;
    orch.add_user("Current", 1)
        .expect("add the current release");
    orch.add_user_on_previous_release("Previous")
        .expect("add the previous release");
    orch.create_all_identities().await.expect("identities");

    orch.exchange_link("Current", "Previous")
        .await
        .expect("link from the current release");

    orch.verify_contact_count("Previous", 1)
        .await
        .expect("Previous has Current");
    orch.verify_contact_count("Current", 1)
        .await
        .expect("Current has Previous");
    orch.stop().await.expect("stop orchestrator");
}
