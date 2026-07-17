// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parsers for the CLI's machine-readable JSON output.

use super::CliDevice;
use crate::device::{CardField, Contact, ContactCard};
use crate::error::{E2eError, E2eResult};

#[derive(Debug, serde::Deserialize)]
struct RawCard {
    display_name: String,
    fields: Vec<RawCardField>,
}

#[derive(Debug, serde::Deserialize)]
struct RawCardField {
    field_type: String,
    label: String,
    value: String,
}

#[derive(Debug, serde::Deserialize)]
struct RawContact {
    id: String,
    display_name: String,
    fingerprint_verified: bool,
    card: RawCard,
}

impl CliDevice {
    pub(super) fn parse_contacts_raw(output: &str) -> E2eResult<Vec<Contact>> {
        let raw: Vec<RawContact> = serde_json::from_str(output).map_err(|e| {
            E2eError::parse_output(format!("Failed to parse 'contacts list --raw' JSON: {e}"))
        })?;

        Ok(raw
            .into_iter()
            .map(|contact| Contact {
                name: contact.display_name,
                id: Some(contact.id),
                verified: contact.fingerprint_verified,
            })
            .collect())
    }

    /// Parse a contact card from the `--raw` JSON output.
    ///
    /// `--raw` is independent of icon tokens, column widths, and terminal
    /// styling, so display changes cannot silently break field assertions.
    pub(super) fn parse_card_raw(output: &str) -> E2eResult<ContactCard> {
        let raw: RawCard = serde_json::from_str(output).map_err(|e| {
            E2eError::parse_output(format!("Failed to parse 'card show --raw' JSON: {e}"))
        })?;

        Ok(Self::contact_card_from_raw(raw))
    }

    pub(super) fn parse_contact_card_raw(output: &str) -> E2eResult<ContactCard> {
        let raw: RawContact = serde_json::from_str(output).map_err(|e| {
            E2eError::parse_output(format!("Failed to parse 'contacts show --raw' JSON: {e}"))
        })?;

        Ok(Self::contact_card_from_raw(raw.card))
    }

    fn contact_card_from_raw(raw: RawCard) -> ContactCard {
        let fields = raw
            .fields
            .into_iter()
            .map(|field| CardField {
                field_type: field.field_type.to_lowercase(),
                label: field.label,
                value: field.value,
            })
            .collect();

        ContactCard {
            name: raw.display_name,
            fields,
        }
    }
}
