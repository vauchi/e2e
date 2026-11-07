// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Device linking for [`super::CoreDevice`]. The initiator and responder
//! stay in memory between steps, where a client process has to save them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use vauchi_core::DeviceSyncOrchestrator;
use vauchi_core::Vauchi;
use vauchi_core::clock::SystemClock;
use vauchi_core::exchange::{
    DeviceLinkInitiator, DeviceLinkQR, DeviceLinkResponder, ProximityProof,
};
use vauchi_core::sync::DeviceLinkIntent;

use super::support::core_error;
use crate::error::{E2eError, E2eResult};

pub(super) struct PendingLink {
    initiator: DeviceLinkInitiator,
}

pub(super) struct PendingJoin {
    responder: DeviceLinkResponder,
    device_name: String,
}

fn now() -> u64 {
    SystemClock::shared().unix_seconds()
}

pub(super) fn start(vauchi: &mut Vauchi) -> E2eResult<(String, PendingLink)> {
    let initiator = vauchi.device_link_initiator().map_err(core_error)?;
    Ok((initiator.qr().to_data_string(), PendingLink { initiator }))
}

pub(super) fn join(qr_data: &str, device_name: &str) -> E2eResult<(String, PendingJoin)> {
    let qr = DeviceLinkQR::from_data_string(qr_data).map_err(core_error)?;
    let mut responder =
        DeviceLinkResponder::from_qr(qr, device_name.to_string(), now()).map_err(core_error)?;
    let request = responder.create_request(now()).map_err(core_error)?;
    Ok((
        BASE64.encode(request),
        PendingJoin {
            responder,
            device_name: device_name.to_string(),
        },
    ))
}

/// The existing device approves the request, as a person who compared the
/// confirmation codes would, and sends its data with the response.
pub(super) fn complete(
    vauchi: &mut Vauchi,
    pending: PendingLink,
    request_data: &str,
) -> E2eResult<String> {
    let initiator = pending.initiator;
    let identity = vauchi
        .identity()
        .ok_or_else(|| E2eError::device("No identity"))?;
    let registry = vauchi
        .storage()
        .device()
        .load_device_registry()
        .map_err(core_error)?
        .unwrap_or_else(|| identity.initial_device_registry());
    let sync_payload = DeviceSyncOrchestrator::new(
        vauchi.storage(),
        identity.create_device_info(now()),
        registry,
    )
    .create_full_sync_payload(DeviceLinkIntent::AddDevice)
    .map_err(core_error)?;
    let sync_json = serde_json::to_string(&sync_payload)
        .map_err(|e| E2eError::device(format!("sync payload: {e}")))?;

    let encrypted_request = BASE64
        .decode(request_data)
        .map_err(|e| E2eError::device(format!("link request is not base64: {e}")))?;
    let (confirmation, request) = initiator
        .prepare_confirmation(&encrypted_request)
        .map_err(core_error)?;
    let proof = ProximityProof::manual_confirmation(
        initiator.qr().link_key(),
        &confirmation.confirmation_code,
        now(),
    );
    let (encrypted_response, updated_registry, _new_device) = initiator
        .confirm_link_with_sync(&request, &sync_json, &proof, now())
        .map_err(core_error)?;
    DeviceSyncOrchestrator::persist_device_registry_change(
        vauchi.storage(),
        identity,
        &updated_registry,
        now(),
    )
    .map_err(core_error)?;
    Ok(BASE64.encode(encrypted_response))
}

pub(super) fn finish(
    vauchi: &mut Vauchi,
    pending: PendingJoin,
    response_data: &str,
) -> E2eResult<()> {
    let PendingJoin {
        responder,
        device_name,
    } = pending;
    let encrypted_response = BASE64
        .decode(response_data)
        .map_err(|e| E2eError::device(format!("link response is not base64: {e}")))?;
    let response = responder
        .process_response(&encrypted_response)
        .map_err(core_error)?;
    vauchi
        .adopt_device_link_response(&response, device_name)
        .map_err(core_error)
}
