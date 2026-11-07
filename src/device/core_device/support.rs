// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Opening a [`super::CoreDevice`]'s `Vauchi`, and the conversions its
//! `Device` methods share.

use std::path::PathBuf;

use vauchi_core::{SymmetricKey, Vauchi, VauchiConfig};

use super::super::{CardField, ContactCard};
use super::{DELAYED_MAX_WAIT, DELAYED_POLL, RELAY_ANCHOR_ENV};
use crate::error::{E2eError, E2eResult};

/// What opening this device's `Vauchi` needs; cloned into blocking tasks.
#[derive(Clone)]
pub(super) struct OpenConfig {
    pub(super) storage_path: PathBuf,
    pub(super) storage_key: SymmetricKey,
    pub(super) relay_url: String,
    pub(super) relay_anchor: Option<[u8; 32]>,
    pub(super) ohttp_relay_url: Option<String>,
}

impl OpenConfig {
    pub(super) fn open(&self) -> E2eResult<Vauchi> {
        let config = VauchiConfig::with_storage_path(&self.storage_path);
        let config = match self.relay_anchor {
            Some(anchor) => config.with_relay(&self.relay_url, anchor),
            None => config.with_relay_url(&self.relay_url),
        };
        let config = match &self.ohttp_relay_url {
            Some(url) => config.with_ohttp_relay_url(url),
            None => config,
        };
        Vauchi::new(config.with_storage_key(self.storage_key.clone())).map_err(core_error)
    }
}

pub(super) fn core_error(error: impl std::fmt::Display) -> E2eError {
    E2eError::device(format!("core: {error}"))
}

/// Runs `work` on a blocking thread: Core's network calls block, and a
/// blocking call on the test's runtime thread would stall the relays' I/O.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> E2eResult<T> + Send + 'static,
) -> E2eResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| E2eError::device(format!("core task failed: {e}")))?
}

pub(super) fn parse_anchor(hex_anchor: &str) -> E2eResult<[u8; 32]> {
    let bytes = hex::decode(hex_anchor.trim())
        .map_err(|e| E2eError::device(format!("{RELAY_ANCHOR_ENV} is not hex: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| E2eError::device(format!("{RELAY_ANCHOR_ENV} is not 32 bytes")))
}

pub(super) fn sync_once(vauchi: &mut Vauchi) -> E2eResult<()> {
    vauchi.connect().map_err(core_error)?;
    let outcome = vauchi
        .sync_collecting_delayed(DELAYED_POLL, DELAYED_MAX_WAIT)
        .map_err(core_error);
    vauchi.disconnect();
    outcome.map(|_| ())
}

pub(super) fn to_e2e_card(card: &vauchi_core::contact_card::ContactCard) -> ContactCard {
    ContactCard {
        name: card.display_name().to_string(),
        fields: card
            .fields()
            .iter()
            .map(|field| CardField {
                field_type: format!("{:?}", field.field_type()).to_lowercase(),
                label: field.label().to_string(),
                value: field.value().to_string(),
            })
            .collect(),
    }
}

pub(super) fn find_contact(
    vauchi: &Vauchi,
    id_or_name: &str,
) -> E2eResult<Option<vauchi_core::Contact>> {
    if let Some(contact) = vauchi.get_contact(id_or_name).map_err(core_error)? {
        return Ok(Some(contact));
    }
    Ok(vauchi
        .find_contact_fuzzy(id_or_name)
        .map_err(core_error)?
        .into_iter()
        .next())
}
