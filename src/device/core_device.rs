// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A device that calls Core's `Vauchi` API in-process instead of a client
//! binary, so Core's merge gate can run the smoke suite against the merge
//! request's Core without building a client (vauchi/private#1118).
//!
//! It opens one `Vauchi` per call from its own data directory, as a client
//! process would, and keeps between calls only what a client keeps between
//! commands: the pending exchange and device-link state.

mod exchange;
mod link;
mod support;

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use tempfile::TempDir;
use vauchi_core::{ContactField, FieldType, SymmetricKey, Vauchi};

use super::{Contact, ContactCard, Device, DeviceType};
use crate::error::{E2eError, E2eResult};
use support::{
    OpenConfig, blocking, core_error, find_contact, parse_anchor, sync_once, to_e2e_card,
};

/// Environment variables the orchestrator sets for a client's relay route.
const OHTTP_RELAY_URL_ENV: &str = "VAUCHI_OHTTP_RELAY_URL";
const RELAY_ANCHOR_ENV: &str = "VAUCHI_RELAY_ANCHOR";

/// Same bound a client uses for the delayed fetch path (cli `config.rs`).
const DELAYED_POLL: std::time::Duration = std::time::Duration::from_secs(30);
const DELAYED_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(960);

const SYNC_RATE_LIMIT_RETRIES: u32 = 3;

/// A device driven through Core's API in this process.
pub struct CoreDevice {
    name: String,
    relay_url: String,
    open_config: OpenConfig,
    /// Password for the identity copies an exchange session takes.
    backup_password: String,
    pending_exchange: Mutex<Option<exchange::PendingExchange>>,
    pending_link: Mutex<Option<link::PendingLink>>,
    pending_join: Mutex<Option<link::PendingJoin>>,
    _data_dir: TempDir,
}

impl CoreDevice {
    /// A device with its own data directory, routed like a client given
    /// `extra_env` (the orchestrator's OHTTP route and relay anchor).
    pub fn new(
        name: impl Into<String>,
        relay_url: impl Into<String>,
        extra_env: &HashMap<String, String>,
    ) -> E2eResult<Self> {
        let data_dir = TempDir::new()
            .map_err(|e| E2eError::device(format!("Failed to create temp directory: {e}")))?;
        let relay_url = relay_url.into();
        let relay_anchor = extra_env
            .get(RELAY_ANCHOR_ENV)
            .map(|hex| parse_anchor(hex))
            .transpose()?;
        let open_config = OpenConfig {
            storage_path: data_dir.path().join("vauchi.db"),
            storage_key: SymmetricKey::generate(),
            relay_url: relay_url.clone(),
            relay_anchor,
            ohttp_relay_url: extra_env.get(OHTTP_RELAY_URL_ENV).cloned(),
        };
        Ok(Self {
            name: name.into(),
            relay_url,
            open_config,
            backup_password: hex::encode(SymmetricKey::generate().as_bytes()),
            pending_exchange: Mutex::new(None),
            pending_link: Mutex::new(None),
            pending_join: Mutex::new(None),
            _data_dir: data_dir,
        })
    }

    async fn with_vauchi<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Vauchi) -> E2eResult<T> + Send + 'static,
    ) -> E2eResult<T> {
        let open_config = self.open_config.clone();
        blocking(move || work(&mut open_config.open()?)).await
    }

    fn not_supported(what: &str) -> E2eError {
        E2eError::DeviceNotSupported(format!("{what} is not implemented on the Core device"))
    }
}

#[async_trait]
impl Device for CoreDevice {
    fn device_type(&self) -> DeviceType {
        DeviceType::Core
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn relay_url(&self) -> &str {
        &self.relay_url
    }

    async fn create_identity(&self, name: &str) -> E2eResult<()> {
        let name = name.to_string();
        self.with_vauchi(move |vauchi| vauchi.create_identity(&name).map_err(core_error))
            .await
    }

    async fn has_identity(&self) -> bool {
        self.with_vauchi(|vauchi| Ok(vauchi.has_identity()))
            .await
            .unwrap_or(false)
    }

    async fn export_identity(&self, _path: &str) -> E2eResult<()> {
        Err(Self::not_supported("export_identity"))
    }

    async fn import_identity(&self, _path: &str) -> E2eResult<()> {
        Err(Self::not_supported("import_identity"))
    }

    async fn generate_qr(&self) -> E2eResult<String> {
        let password = self.backup_password.clone();
        let (qr, pending) = self
            .with_vauchi(move |vauchi| exchange::start(vauchi, &password))
            .await?;
        *self.pending_exchange.lock().unwrap() = Some(pending);
        Ok(qr)
    }

    async fn complete_exchange(&self, qr_data: &str) -> E2eResult<()> {
        let pending = self
            .pending_exchange
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| {
                E2eError::device("No pending exchange: call generate_qr before complete_exchange")
            })?;
        let password = self.backup_password.clone();
        let qr_data = qr_data.to_string();
        self.with_vauchi(move |vauchi| exchange::complete(vauchi, &password, pending, &qr_data))
            .await
    }

    async fn start_device_link(&self) -> E2eResult<String> {
        let (qr, pending) = self.with_vauchi(link::start).await?;
        *self.pending_link.lock().unwrap() = Some(pending);
        Ok(qr)
    }

    async fn join_identity(&self, qr_data: &str, device_name: &str) -> E2eResult<String> {
        let (request, pending) = link::join(qr_data, device_name)?;
        *self.pending_join.lock().unwrap() = Some(pending);
        Ok(request)
    }

    async fn complete_device_link(&self, request_data: &str) -> E2eResult<String> {
        let pending = self.pending_link.lock().unwrap().take().ok_or_else(|| {
            E2eError::device("No pending device link: call start_device_link first")
        })?;
        let request_data = request_data.to_string();
        self.with_vauchi(move |vauchi| link::complete(vauchi, pending, &request_data))
            .await
    }

    async fn finish_device_join(&self, response_data: &str) -> E2eResult<()> {
        let pending =
            self.pending_join.lock().unwrap().take().ok_or_else(|| {
                E2eError::device("No pending device join: call join_identity first")
            })?;
        let response_data = response_data.to_string();
        self.with_vauchi(move |vauchi| link::finish(vauchi, pending, &response_data))
            .await
    }

    async fn list_devices(&self) -> E2eResult<Vec<String>> {
        self.with_vauchi(|vauchi| {
            let registry = vauchi
                .storage()
                .device()
                .load_device_registry()
                .map_err(core_error)?;
            Ok(registry
                .map(|registry| {
                    registry
                        .all_devices()
                        .iter()
                        .map(|device| device.device_name.clone())
                        .collect()
                })
                .unwrap_or_default())
        })
        .await
    }

    async fn sync(&self) -> E2eResult<()> {
        for attempt in 0..=SYNC_RATE_LIMIT_RETRIES {
            match self.with_vauchi(sync_once).await {
                Err(error)
                    if attempt < SYNC_RATE_LIMIT_RETRIES
                        && error.to_string().contains("Rate limited") =>
                {
                    // The test relay's per-client token bucket refills over
                    // seconds; the CLI device waits the same way.
                    tokio::time::sleep(std::time::Duration::from_secs(10 + 2 * attempt as u64))
                        .await;
                }
                other => return other,
            }
        }
        unreachable!("the last attempt always returns")
    }

    async fn list_contacts(&self) -> E2eResult<Vec<Contact>> {
        self.with_vauchi(|vauchi| {
            Ok(vauchi
                .list_contacts()
                .map_err(core_error)?
                .iter()
                .map(|contact| Contact {
                    name: contact.display_name().to_string(),
                    id: Some(contact.id().to_string()),
                    verified: contact.is_fingerprint_verified(),
                })
                .collect())
        })
        .await
    }

    async fn get_contact(&self, name_or_id: &str) -> E2eResult<Option<Contact>> {
        let query = name_or_id.to_string();
        self.with_vauchi(move |vauchi| {
            Ok(find_contact(vauchi, &query)?.map(|contact| Contact {
                name: contact.display_name().to_string(),
                id: Some(contact.id().to_string()),
                verified: contact.is_fingerprint_verified(),
            }))
        })
        .await
    }

    async fn get_card(&self) -> E2eResult<ContactCard> {
        self.with_vauchi(|vauchi| {
            let card = vauchi
                .own_card()
                .map_err(core_error)?
                .ok_or_else(|| E2eError::device("No own card"))?;
            Ok(to_e2e_card(&card))
        })
        .await
    }

    async fn add_field(&self, field_type: &str, label: &str, value: &str) -> E2eResult<()> {
        let (field_type, label, value) =
            (field_type.to_string(), label.to_string(), value.to_string());
        self.with_vauchi(move |vauchi| {
            let kind = FieldType::from_alias(&field_type, &vauchi_app::social::active_registry())
                .map(|(kind, _hint)| kind)
                .ok_or_else(|| E2eError::device(format!("Unknown field type: {field_type}")))?;
            let old_card = vauchi
                .own_card()
                .map_err(core_error)?
                .ok_or_else(|| E2eError::device("No own card"))?;
            let now = vauchi.clock().unix_seconds();
            vauchi
                .add_own_field(ContactField::new(kind, &label, &value, now))
                .map_err(core_error)?;
            let new_card = vauchi
                .own_card()
                .map_err(core_error)?
                .ok_or_else(|| E2eError::device("No own card"))?;
            vauchi
                .propagate_card_update(&old_card, &new_card)
                .map_err(core_error)?;
            Ok(())
        })
        .await
    }

    async fn edit_field(&self, _label: &str, _value: &str) -> E2eResult<()> {
        Err(Self::not_supported("edit_field"))
    }

    async fn remove_field(&self, _label: &str) -> E2eResult<()> {
        Err(Self::not_supported("remove_field"))
    }

    async fn edit_name(&self, _name: &str) -> E2eResult<()> {
        Err(Self::not_supported("edit_name"))
    }

    async fn create_label(&self, name: &str) -> E2eResult<()> {
        let name = name.to_string();
        self.with_vauchi(move |vauchi| vauchi.create_group(&name).map(|_| ()).map_err(core_error))
            .await
    }

    async fn list_labels(&self) -> E2eResult<Vec<String>> {
        self.with_vauchi(|vauchi| {
            Ok(vauchi
                .list_groups()
                .map_err(core_error)?
                .iter()
                .map(|group| group.name().to_string())
                .collect())
        })
        .await
    }

    async fn add_contact_to_label(&self, label: &str, contact: &str) -> E2eResult<()> {
        let (label, contact) = (label.to_string(), contact.to_string());
        self.with_vauchi(move |vauchi| {
            let group = vauchi
                .list_groups()
                .map_err(core_error)?
                .into_iter()
                .find(|group| group.name() == label)
                .ok_or_else(|| E2eError::device(format!("Label not found: {label}")))?;
            let contact = find_contact(vauchi, &contact)?
                .ok_or_else(|| E2eError::device(format!("Contact not found: {contact}")))?;
            vauchi
                .add_contact_to_group(group.id(), contact.id())
                .map_err(core_error)
        })
        .await
    }

    async fn hide_field_from_contact(&self, contact: &str, field: &str) -> E2eResult<()> {
        let (contact, field) = (contact.to_string(), field.to_string());
        self.with_vauchi(move |vauchi| {
            let contact = find_contact(vauchi, &contact)?
                .ok_or_else(|| E2eError::device(format!("Contact not found: {contact}")))?;
            let field_id = vauchi.resolve_own_field_id(&field).map_err(core_error)?;
            vauchi
                .set_contact_visibility_override_and_repropagate(contact.id(), &field_id, false)
                .map_err(core_error)
        })
        .await
    }
}
