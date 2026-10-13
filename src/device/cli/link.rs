// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Link-mode exchange for CLI users (private#670): the sharing side runs
//! `vauchi exchange link` in the background until the other side accepts.

use super::CliDevice;
use crate::error::{E2eError, E2eResult};

/// Longest a Link-mode exchange may take: the CLI's own 300 s wait
/// (ADR-035's link window) plus a margin for start-up.
const LINK_EXCHANGE_BUDGET: std::time::Duration = std::time::Duration::from_secs(330);

impl CliDevice {
    /// Start `vauchi exchange link`, return the link it prints and keep the
    /// process running.
    pub(super) async fn spawn_link_share(&self) -> E2eResult<String> {
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
        let url = tokio::time::timeout(self.command_timeout, async {
            while let Some(line) = lines.next_line().await? {
                if let Some(url) = line
                    .split_whitespace()
                    .find(|word| word.starts_with("https://vauchi.app/x#"))
                {
                    return Ok(Some(url.to_string()));
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
        Ok(url)
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
