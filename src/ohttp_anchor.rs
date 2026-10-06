// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A test anchor for each spawned relay (#288 plan 5.1), made with the same
//! `vauchi-ohttp-anchor` ceremony an operator runs, so e2e exercises the
//! real tool and the relay signs under an anchor the test knows.
//!
//! Without it a windowed relay would anchor itself in its key directory,
//! which several relays of one run share.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::{E2eError, E2eResult};
use crate::relay_manager::RelayInstance;

const ANCHOR_BINARY: &str = "vauchi-ohttp-anchor";

/// A relay's anchor public key and the intermediate files it signs with.
#[derive(Debug, Clone)]
pub struct TestAnchor {
    pub public_key: [u8; 32],
    key_path: PathBuf,
    certs_path: PathBuf,
}

impl TestAnchor {
    /// Run the ceremony in `dir`: a new anchor, a new intermediate key, and
    /// the anchor's two certificates for it. The anchor seed lives only in
    /// this process's memory, as on the operator's machine.
    pub async fn provision(relay_binary: &Path, dir: &Path) -> E2eResult<Self> {
        let cli = anchor_binary(relay_binary)?;
        std::fs::create_dir_all(dir)
            .map_err(|e| E2eError::relay(format!("OHTTP anchor workspace: {e}")))?;
        let key_path = dir.join("ohttp-intermediate.key");
        let certs_path = dir.join("ohttp-intermediate.certs");

        let seed = run(&cli, &["create"], b"").await?;
        let public_key = parse_key(&run(&cli, &["public-key"], seed.as_bytes()).await?)?;
        let intermediate = run(&cli, &["intermediate-key", path_str(&key_path)?], b"").await?;
        run(
            &cli,
            &["sign", intermediate.trim(), path_str(&certs_path)?],
            seed.as_bytes(),
        )
        .await?;

        Ok(Self {
            public_key,
            key_path,
            certs_path,
        })
    }

    /// The relay settings that make it sign with this anchor's intermediate.
    pub fn insert_env(&self, env: &mut HashMap<String, String>) {
        env.insert(
            "RELAY_OHTTP_INTERMEDIATE_KEY_PATH".to_string(),
            self.key_path.to_string_lossy().into_owned(),
        );
        env.insert(
            "RELAY_OHTTP_INTERMEDIATE_CERTS_PATH".to_string(),
            self.certs_path.to_string_lossy().into_owned(),
        );
    }
}

/// The ceremony CLI is built with the relay, so it sits next to it.
fn anchor_binary(relay_binary: &Path) -> E2eResult<PathBuf> {
    let path = relay_binary.with_file_name(ANCHOR_BINARY);
    if path.exists() {
        Ok(path)
    } else {
        Err(E2eError::relay(format!(
            "{ANCHOR_BINARY} not found next to {}; rebuild the relay (it ships both binaries)",
            relay_binary.display()
        )))
    }
}

async fn run(cli: &Path, args: &[&str], stdin: &[u8]) -> E2eResult<String> {
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| E2eError::relay(format!("{ANCHOR_BINARY} {}: {e}", args[0])))?;
    if let Some(mut input) = child.stdin.take() {
        input
            .write_all(stdin)
            .await
            .map_err(|e| E2eError::relay(format!("{ANCHOR_BINARY} {} stdin: {e}", args[0])))?;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| E2eError::relay(format!("{ANCHOR_BINARY} {}: {e}", args[0])))?;
    if !output.status.success() {
        return Err(E2eError::relay(format!(
            "{ANCHOR_BINARY} {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|out| out.trim().to_owned())
        .map_err(|e| E2eError::relay(format!("{ANCHOR_BINARY} {} output: {e}", args[0])))
}

fn parse_key(hex_key: &str) -> E2eResult<[u8; 32]> {
    let mut key = [0u8; 32];
    hex::decode_to_slice(hex_key, &mut key)
        .map_err(|e| E2eError::relay(format!("anchor public key is not 64 hex characters: {e}")))?;
    Ok(key)
}

fn path_str(path: &Path) -> E2eResult<&str> {
    path.to_str()
        .ok_or_else(|| E2eError::relay(format!("non-UTF-8 path {}", path.display())))
}

/// The e2e OHTTP controls of a spawned relay.
impl RelayInstance {
    /// The anchor this relay signs its OHTTP keys under, as a client would
    /// hold it. `None` for an interval-mode relay.
    pub fn ohttp_anchor(&self) -> Option<[u8; 32]> {
        self.ohttp_anchor.as_ref().map(|anchor| anchor.public_key)
    }

    /// The relay's OHTTP clock, in Unix seconds, as e2e last set it.
    pub fn ohttp_clock(&self) -> u64 {
        self.ohttp_clock.load(Ordering::SeqCst)
    }

    /// Move the relay's OHTTP gateway `windows` 24 h windows on, a minute
    /// past the boundary. Needs a relay built with `e2e-test-clock`.
    /// Returns the new clock.
    pub async fn advance_ohttp_windows(&self, windows: u64) -> E2eResult<u64> {
        let target =
            (self.ohttp_clock() / OHTTP_WINDOW_SECONDS + windows) * OHTTP_WINDOW_SECONDS + 60;
        let response = reqwest::Client::new()
            .post(format!("{}/__e2e/clock", self.http_url()))
            .body(target.to_string())
            .send()
            .await
            .map_err(|e| E2eError::relay(format!("setting the relay's OHTTP clock: {e}")))?;
        match response.status() {
            reqwest::StatusCode::OK => {
                self.ohttp_clock.store(target, Ordering::SeqCst);
                Ok(target)
            }
            reqwest::StatusCode::NOT_FOUND => Err(E2eError::relay(
                "the relay has no /__e2e/clock: build it with --features e2e-test-clock",
            )),
            status => Err(E2eError::relay(format!(
                "setting the relay's OHTTP clock to {target} answered {status}"
            ))),
        }
    }
}

const OHTTP_WINDOW_SECONDS: u64 = 86_400;

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
