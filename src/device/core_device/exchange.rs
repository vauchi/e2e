// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! QR exchange for [`super::CoreDevice`]: both sides show a QR, then each
//! completes with the other's, as two people at a table would.

use vauchi_core::Identity;
use vauchi_core::Vauchi;
use vauchi_core::clock::SystemClock;
use vauchi_core::contact_card::ContactCard;
use vauchi_core::exchange::{
    ExchangeEvent, ExchangeQR, ExchangeSession, ExchangeState, ManualConfirmationVerifier,
};
use zeroize::Zeroizing;

use super::support::core_error;
use crate::error::{E2eError, E2eResult};

/// What `start` hands `complete`: the session's resume secret and its QR.
pub(super) struct PendingExchange {
    resume_secret: Zeroizing<[u8; 32]>,
    our_qr: String,
}

/// The session takes an `Identity` of its own, and `Identity` is not
/// `Clone`, so it is copied through an encrypted backup.
fn owned_identity(vauchi: &Vauchi, password: &str) -> E2eResult<(Identity, ContactCard)> {
    let identity = vauchi
        .identity()
        .ok_or_else(|| E2eError::device("No identity"))?;
    let card = vauchi
        .own_card()
        .map_err(core_error)?
        .unwrap_or_else(|| ContactCard::new(identity.display_name()));
    let backup = identity.export_backup(password).map_err(core_error)?;
    let now = SystemClock::shared().unix_seconds();
    let copy = Identity::import_backup(&backup, password, now).map_err(core_error)?;
    Ok((copy, card))
}

pub(super) fn start(vauchi: &mut Vauchi, password: &str) -> E2eResult<(String, PendingExchange)> {
    let (identity, card) = owned_identity(vauchi, password)?;
    let mut session = ExchangeSession::new_qr(
        identity,
        card,
        ManualConfirmationVerifier::new(),
        SystemClock::shared(),
    );
    session.apply(ExchangeEvent::StartQR).map_err(debug_error)?;
    let our_qr = session
        .qr()
        .ok_or_else(|| E2eError::device("QR not generated"))?
        .to_data_string();
    let resume_secret = session
        .qr_resume_secret()
        .ok_or_else(|| E2eError::device("QR session has no resume secret"))?;
    Ok((
        our_qr.clone(),
        PendingExchange {
            resume_secret,
            our_qr,
        },
    ))
}

pub(super) fn complete(
    vauchi: &mut Vauchi,
    password: &str,
    pending: PendingExchange,
    their_qr: &str,
) -> E2eResult<()> {
    let their_qr = ExchangeQR::from_data_string(their_qr).map_err(core_error)?;
    let our_qr = ExchangeQR::from_data_string(&pending.our_qr).map_err(core_error)?;
    let (identity, card) = owned_identity(vauchi, password)?;
    let mut session = ExchangeSession::resume_qr(
        identity,
        card,
        ManualConfirmationVerifier::new(),
        *pending.resume_secret,
        our_qr,
        SystemClock::shared(),
    )
    .map_err(debug_error)?;

    for event in [
        ExchangeEvent::ProcessQR(their_qr),
        ExchangeEvent::TheyScannedOurQR,
        ExchangeEvent::PerformKeyAgreement,
    ] {
        session.apply(event).map_err(debug_error)?;
    }
    let their_name = session
        .their_display_name()
        .filter(|name| !name.is_empty())
        .unwrap_or("New Contact")
        .to_string();
    session
        .apply(ExchangeEvent::CompleteExchange(ContactCard::new(
            &their_name,
        )))
        .map_err(debug_error)?;

    let contact = match session.state() {
        ExchangeState::Complete { contact } => *contact.clone(),
        _ => return Err(E2eError::device("Exchange did not complete")),
    };
    let (ratchet, is_initiator) = session
        .build_exchange_ratchet(&contact)
        .map_err(debug_error)?;
    vauchi
        .save_exchanged_contact(&contact, &ratchet, is_initiator)
        .map_err(core_error)?;

    // The first card gives the responder its receive chain, so both sides
    // can send updates. A card with no fields yet has nothing to queue
    // ("empty delta"); then, as in cli `exchange complete`, nothing is sent
    // now and the exchange still stands.
    match vauchi.queue_initial_card_for_contact(contact.id()) {
        Ok(()) => super::support::sync_once(vauchi),
        Err(error) => {
            tracing::debug!("initial card not queued: {error}");
            Ok(())
        }
    }
}

fn debug_error(error: impl std::fmt::Debug) -> E2eError {
    E2eError::device(format!("core: {error:?}"))
}
