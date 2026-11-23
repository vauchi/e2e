// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A device's notice-core session through the real relay and OHTTP relay
//! binaries and back (ADR-081, proposed; vauchi/private#1281): both devices
//! register, the sender writes every slot of one session over OHTTP, the
//! period closes, the receiver fetches the batch and installs the card.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use vauchi_core::crypto::X25519KeyPair;
use vauchi_core::network::http_transport::HttpTransport;
use vauchi_core::notice::content::{RECORD_BYTES, commitment};
use vauchi_core::notice::direction::{Receiver, Sender};
use vauchi_core::notice::keys::{Label, seed_epoch};
use vauchi_core::notice::publish::{Pending, Publisher};
use vauchi_core::notice::seal::NOTICE_BYTES;
use vauchi_core::notice::session::{FAST, Session, records_in_chunk};
use vauchi_e2e_tests::ohttp_relay_manager::{OhttpRelayConfig, OhttpRelayManager};
use vauchi_e2e_tests::relay_manager::{RelayConfig, RelayManager};
use vauchi_protocol::cohort::{
    CohortBatchRequest, CohortChunkRequest, CohortClass, CohortRegisterRequest, CohortTier,
};

use super::ohttp_helpers::{create_ohttp_transport, signed_key_config};

const COHORT: u32 = 7;

fn pair(now: u64) -> (Sender, Receiver) {
    let sender_device = X25519KeyPair::generate();
    let receiver_device = X25519KeyPair::generate();
    let label = Label::new(sender_device.public_key(), receiver_device.public_key());
    let handshake = X25519KeyPair::generate()
        .diffie_hellman(X25519KeyPair::generate().public_key())
        .expect("contributory");
    let sender_public = *sender_device.public_key();
    let receiver_public = *receiver_device.public_key();
    (
        Sender::new(
            seed_epoch(&handshake, &label),
            label.clone(),
            sender_device,
            receiver_public,
            now,
        ),
        Receiver::new(
            seed_epoch(&handshake, &label),
            label,
            receiver_device,
            sender_public,
            now,
        ),
    )
}

/// Every record of one class in the batch of the period holding `at`.
fn fetch(http: &HttpTransport, class: CohortClass, at: u64) -> (usize, Vec<Vec<u8>>) {
    let batch = http
        .cohort_batch(&CohortBatchRequest {
            cohort: COHORT,
            tier: CohortTier::Fast,
            class,
            at,
        })
        .expect("the batch answers");
    assert!(batch.is_ok(), "batch: {batch:?}");
    let records = batch.records.expect("a record count");
    let period = batch.period.expect("the period");
    let chunks = (0..batch.chunks.expect("a chunk count"))
        .map(|index| {
            let chunk = http
                .cohort_chunk(&CohortChunkRequest {
                    cohort: COHORT,
                    tier: CohortTier::Fast,
                    class,
                    period,
                    index,
                })
                .expect("the chunk answers");
            assert!(chunk.is_ok(), "chunk {index}: {chunk:?}");
            STANDARD
                .decode(chunk.chunk.expect("chunk bytes"))
                .expect("base64")
        })
        .collect();
    (records, chunks)
}

// @internal
#[tokio::test]
async fn a_session_written_over_ohttp_comes_back_and_the_receiver_installs_the_card() {
    let mut relay_mgr = RelayManager::with_config(RelayConfig {
        http_api_enabled: true,
        ohttp_enabled: true,
        cohort_store: Some("memory".to_string()),
        ..Default::default()
    })
    .await
    .expect("relay manager");
    relay_mgr.spawn(1).await.expect("spawn relay");
    let relay_url = relay_mgr.relay(0).expect("relay").http_url();
    let mut ohttp_mgr = OhttpRelayManager::new(OhttpRelayConfig::default()).expect("ohttp relay");
    ohttp_mgr
        .spawn(&relay_url)
        .await
        .expect("spawn ohttp-relay");
    let ohttp_url = ohttp_mgr.url().expect("ohttp relay url");
    let relay = relay_mgr.relay(0).expect("relay");
    // A minute into a fresh OHTTP window, so no key rotates mid-test.
    let now = relay.advance_ohttp_windows(1).await.expect("relay clock");
    let key = signed_key_config(&ohttp_url, relay).await;
    let http = create_ohttp_transport(&ohttp_url, &key);

    let sender_device = [0x5A; 32];
    let receiver_device = [0xA5; 32];
    for device in [sender_device, receiver_device] {
        let registered = http
            .cohort_register(&CohortRegisterRequest {
                cohort: COHORT,
                device: STANDARD.encode(device),
            })
            .expect("registration answers");
        assert!(registered.is_ok(), "register: {registered:?}");
    }

    let (mut sender, mut receiver) = pair(now);
    let card = b"Alice, alice@example.org, +41 00 000 00 00".to_vec();
    let key = [0x31; 32];
    sender
        .queue_group_update(3, &key, &commitment(&key, &card))
        .expect("within 32 groups");
    let mut publishers = vec![Publisher::for_group(3, key, &card)];
    let written = Session::default().write(&mut [&mut sender], &mut [], &mut publishers, FAST, now);
    let requests = written.cohort_writes(COHORT, &sender_device, CohortTier::Fast);
    assert_eq!(requests.len(), FAST.notices + FAST.contents);
    for request in &requests {
        let answer = http.cohort_write(request).expect("the write answers");
        assert!(answer.is_ok(), "slot {}: {answer:?}", request.slot);
    }

    let early = http
        .cohort_batch(&CohortBatchRequest {
            cohort: COHORT,
            tier: CohortTier::Fast,
            class: CohortClass::Notice,
            at: now,
        })
        .expect("the batch answers");
    assert_eq!(
        early.error.as_deref(),
        Some("not_published"),
        "nothing is served before the period ends"
    );
    relay
        .set_ohttp_clock(now + 2 * 60)
        .await
        .expect("the period ends");

    let (notice_count, notice_chunks) = fetch(&http, CohortClass::Notice, now);
    assert_eq!(
        notice_count, FAST.notices,
        "one device wrote in this period"
    );
    let notices: Vec<[u8; NOTICE_BYTES]> = notice_chunks
        .iter()
        .enumerate()
        .flat_map(|(index, chunk)| {
            records_in_chunk::<NOTICE_BYTES>(notice_count, index, chunk).expect("a chunk")
        })
        .collect();
    let (content_count, content_chunks) = fetch(&http, CohortClass::Content, now);
    assert_eq!(content_count, FAST.contents);
    let contents: Vec<[u8; RECORD_BYTES]> = content_chunks
        .iter()
        .enumerate()
        .flat_map(|(index, chunk)| {
            records_in_chunk::<RECORD_BYTES>(content_count, index, chunk).expect("a chunk")
        })
        .collect();

    let read_at = now + 3 * 60;
    let mut pending = Pending::default();
    for read in receiver.read(&notices, read_at) {
        pending.note(&read.update_key, &read.commitment, read_at);
    }
    let installed = pending.read(&contents, read_at, |_| true);
    assert_eq!(installed.len(), 1, "the card");
    assert_eq!(installed[0].as_slice(), card.as_slice());

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}
