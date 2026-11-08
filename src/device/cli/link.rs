// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Link-mode exchange for CLI users (private#670): the sharing side runs
//! `vauchi exchange link` in the background until the other side accepts.

use super::CliDevice;
use crate::device::ExchangeLink;
use crate::error::{E2eError, E2eResult};

/// Longest a Link-mode exchange may take: the CLI's own 300 s wait
/// (ADR-035's link window) plus a margin for start-up.
const LINK_EXCHANGE_BUDGET: std::time::Duration = std::time::Duration::from_secs(330);

impl CliDevice {
    /// The code a sharing `vauchi exchange link` prints before its link
    /// (`Confirmation code: 123 456`), as the digits the other side types.
    pub fn shared_link_code(line: &str) -> Option<String> {
        let digits: String = line
            .strip_prefix("Confirmation code:")?
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit())).then_some(digits)
    }

    /// Start `vauchi exchange link`, return the link and code it prints and
    /// keep the process running.
    pub(super) async fn spawn_link_share(&self) -> E2eResult<ExchangeLink> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = self
            .cli_command(&["exchange", "link"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| E2eError::cli_execution(format!("spawn vauchi exchange link: {e}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| E2eError::cli_execution("exchange link: no stdout"))?;
        let mut lines = BufReader::new(stdout).lines();
        let link = tokio::time::timeout(self.command_timeout, async {
            let mut code = None;
            while let Some(line) = lines.next_line().await? {
                if let Some(shown) = Self::shared_link_code(&line) {
                    code = Some(shown);
                }
                if let Some(url) = line
                    .split_whitespace()
                    .find(|word| word.starts_with("https://vauchi.app/x#"))
                {
                    return Ok(Some(ExchangeLink {
                        url: url.to_string(),
                        code,
                    }));
                }
            }
            Ok::<_, std::io::Error>(None)
        })
        .await
        .map_err(|_| E2eError::timeout("exchange link printed no link"))?
        .map_err(|e| E2eError::cli_execution(format!("read exchange link output: {e}")))?
        .ok_or_else(|| E2eError::cli_execution("exchange link exited without a link"))?;
        // Keep draining, or the process meets a closed pipe on its next line.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        *self.link_initiator.lock().await = Some(child);
        Ok(link)
    }

    /// Accept the other side's link, typing its code on standard input when
    /// the sharer showed one; a CLI on Core 0.101.0 refuses a link without it.
    pub(super) async fn accept_link(&self, link: &ExchangeLink) -> E2eResult<()> {
        use tokio::io::AsyncWriteExt;

        let Some(code) = &link.code else {
            self.run_command_success(&["exchange", "link", &link.url, "--yes"])
                .await?;
            return Ok(());
        };
        let mut child = self
            .cli_command(&["exchange", "link", &link.url, "--yes", "--code-stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| E2eError::cli_execution(format!("spawn vauchi exchange link: {e}")))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| E2eError::cli_execution("exchange link: no stdin"))?;
        stdin
            .write_all(format!("{code}\n").as_bytes())
            .await
            .map_err(|e| E2eError::cli_execution(format!("type the link code: {e}")))?;
        drop(stdin);
        let output = tokio::time::timeout(LINK_EXCHANGE_BUDGET, child.wait_with_output())
            .await
            .map_err(|_| E2eError::timeout("exchange link did not finish"))?
            .map_err(|e| E2eError::cli_execution(format!("wait for exchange link: {e}")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(E2eError::cli_execution(format!(
                "exchange link with code failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    /// Wait for the sharing process to finish; success means the exchange
    /// completed.
    pub(super) async fn wait_link_share(&self) -> E2eResult<()> {
        let child = self
            .link_initiator
            .lock()
            .await
            .take()
            .ok_or_else(|| E2eError::device("no link exchange in flight"))?;
        let output = tokio::time::timeout(LINK_EXCHANGE_BUDGET, child.wait_with_output())
            .await
            .map_err(|_| E2eError::timeout("exchange link did not finish"))?
            .map_err(|e| E2eError::cli_execution(format!("wait for exchange link: {e}")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(E2eError::cli_execution(format!(
                "exchange link failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}
