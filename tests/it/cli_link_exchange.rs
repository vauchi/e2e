// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::feature_driver::{Backend, run};

// @scenario: link_exchange :: Two users exchange contact cards using a link
// @scenario: link_exchange :: A malformed exchange link is rejected without adding contacts
// @scenario: link_exchange :: Users exchange contact cards after a malformed link is rejected
#[tokio::test]
async fn integration_cli_to_cli_link_exchange() {
    run(Backend::Cli).await;
}
