// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::TuiDevice;
use crate::device::ExchangeLinkFeedback;
use crate::error::E2eResult;
use std::time::Duration;

impl TuiDevice {
    pub(super) async fn link_feedback(&self, url: &str) -> E2eResult<ExchangeLinkFeedback> {
        self.session.ensure_started().await?;
        self.session.navigate_to("Exchange").await?;
        self.session
            .wait_for_frame("Exchange Mode|Pick a way", Duration::from_secs(10))
            .await?;
        self.session.send_char('1').await?;
        self.session
            .wait_for_frame("Share Link|Send this link", Duration::from_secs(10))
            .await?;
        self.session.send_text(url).await?;
        self.session
            .wait_for_frame(url, Duration::from_secs(10))
            .await?;
        self.session.send_enter().await?;
        let feedback = self
            .session
            .wait_for_frame("Invalid Link|Exchange Request", Duration::from_secs(10))
            .await?;
        if feedback != "Invalid Link" {
            return Ok(ExchangeLinkFeedback::Other(feedback));
        }
        self.session
            .wait_for_frame(
                "This link cannot be opened in Vauchi.",
                Duration::from_secs(10),
            )
            .await?;
        self.session.send_escape().await?;
        self.session
            .send_text(&"\u{7f}".repeat(url.chars().count()))
            .await?;
        Ok(ExchangeLinkFeedback::InvalidLink)
    }
}
