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

    /// Paste the peer's link on the Link share screen, accept the exchange
    /// request Core raises for it, and type the code when the sharer showed
    /// one: a TUI on Core 0.101.0 asks for it before it retrieves the card
    /// (vauchi/private#727; camera-less route,
    /// `problems/2026-09-09-tui-cannot-ingest-peer-exchange-payload`).
    pub(super) async fn open_link(&self, link: &str, code: Option<&str>) -> E2eResult<()> {
        self.session.ensure_started().await?;
        self.session.navigate_to("Exchange").await?;
        self.session
            .wait_for_frame("Exchange Mode|Pick a way", Duration::from_secs(10))
            .await?;
        self.session.send_char('1').await?;
        self.session
            .wait_for_frame("Share Link|Send this link", Duration::from_secs(10))
            .await?;
        // Typing lands in the first input (the peer-link field), and Return
        // submits it: Core routes the pasted link like an opened deep link.
        self.session.send_text(link).await?;
        self.session
            .wait_for_frame(link, Duration::from_secs(10))
            .await?;
        self.session.send_enter().await?;
        self.session
            .wait_for_visible("Exchange Request|Accept Exchange", Duration::from_secs(20))
            .await?;
        // Return with no focused input activates the context-bar primary
        // ("Accept Exchange").
        self.session.send_enter().await?;
        if let Some(code) = code {
            self.session
                .wait_for_visible("6-digit code", Duration::from_secs(20))
                .await?;
            self.session.send_text(code).await?;
            self.session.send_enter().await?;
        }
        self.session
            .wait_for_visible(
                "Contact added|Contact Added|Exchange complete|Exchange Complete|Contacts",
                Duration::from_secs(60),
            )
            .await?;
        Ok(())
    }
}
