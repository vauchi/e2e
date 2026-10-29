// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A hard shred a week after its soft shred must purge on a real relay.
//!
//! Soft shred stores a pre-signed purge; the hard shred runs at least the
//! 7-day grace period later. The relay accepts a purge only within 60 s of
//! its clock, so a shred that sends the stored purge is refused and the
//! relay keeps the user's data (vauchi/private#899). The CLI signs its own
//! purge and never writes the stored one, so only Core's `ShredManager`
//! path, the one the mobile apps take, shows this; the test drives it
//! directly against a spawned relay.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use vauchi_core::api::{PreSignedPurgeRequest, PurgeSender, ShredError, ShredManager};
use vauchi_core::clock::Clock;
use vauchi_core::crypto::SymmetricKey;
use vauchi_core::identity::Identity;
use vauchi_core::network::{HttpTransport, HttpTransportConfig, ProxyConfig};
use vauchi_core::storage::{SecureStorage, Storage, StorageError};
use vauchi_e2e_tests::relay_manager::{RelayConfig, RelayManager};

const EIGHT_DAYS: Duration = Duration::from_secs(8 * 24 * 60 * 60);

/// Reads eight days in the past until `arrive` is called, then real time:
/// the soft shred happens a week before the hard shred, which meets the
/// relay at its own (real) clock.
#[derive(Default)]
struct GraceWeekClock {
    arrived: AtomicBool,
}

impl GraceWeekClock {
    fn arrive(&self) {
        self.arrived.store(true, Ordering::SeqCst);
    }
}

impl Clock for GraceWeekClock {
    fn now(&self) -> SystemTime {
        if self.arrived.load(Ordering::SeqCst) {
            SystemTime::now()
        } else {
            SystemTime::now() - EIGHT_DAYS
        }
    }
}

#[derive(Default)]
struct InMemoryKeys(Mutex<HashMap<String, Vec<u8>>>);

impl SecureStorage for InMemoryKeys {
    fn save_key(&self, name: &str, key: &[u8]) -> Result<(), StorageError> {
        self.0
            .lock()
            .unwrap()
            .insert(name.to_string(), key.to_vec());
        Ok(())
    }

    fn load_key(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }

    fn delete_key(&self, name: &str) -> Result<(), StorageError> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// Sends the purge the way the HTTP transport adapter does and keeps the
/// relay's answer.
struct RelayPurge {
    http: HttpTransport,
    answers: Vec<Result<(), String>>,
}

impl PurgeSender for RelayPurge {
    fn send_purge(&mut self, purge: &PreSignedPurgeRequest, _now: u64) -> Result<bool, ShredError> {
        let public_key = hex::encode(purge.public_key);
        let answer = self
            .http
            .purge(
                &public_key,
                &public_key,
                &hex::encode(purge.purge_token),
                &hex::encode(&purge.signature),
                purge.timestamp,
            )
            .map(|_| ())
            .map_err(|e| e.to_string());
        let accepted = answer.is_ok();
        self.answers.push(answer);
        Ok(accepted)
    }
}

// @internal
#[tokio::test]
async fn a_hard_shred_a_week_after_the_soft_shred_purges_on_a_real_relay() {
    let mut relay = RelayManager::with_config(RelayConfig {
        http_api_enabled: true,
        ohttp_enabled: false,
        ..Default::default()
    })
    .await
    .expect("relay manager");
    relay.spawn(1).await.expect("spawn relay");
    let relay_url = relay.relay(0).expect("relay instance").http_url();

    let dir = tempfile::tempdir().expect("data dir");
    let clock = Arc::new(GraceWeekClock::default());
    let storage = Storage::open(dir.path().join("vauchi.db"), SymmetricKey::generate())
        .expect("storage")
        .with_clock(clock.clone());
    let keys = InMemoryKeys::default();
    let me = Identity::create("Me", 0);
    let shred = ShredManager::new(&storage, &keys, &me, dir.path());

    let token = shred.soft_shred().expect("soft shred");
    clock.arrive();
    let mut purge = RelayPurge {
        http: HttpTransport::new(HttpTransportConfig {
            relay_url,
            timeout_ms: 10_000,
            proxy: ProxyConfig::None,
            allow_direct: true,
            pinned_certs: vec![],
        }),
        answers: Vec::new(),
    };
    let report = shred
        .hard_shred(token, Some(&mut purge), None)
        .expect("hard shred");

    assert_eq!(
        purge.answers,
        vec![Ok(())],
        "the relay's answer to the purge"
    );
    assert!(report.relay_purge_sent);

    relay.stop_all().await;
}
