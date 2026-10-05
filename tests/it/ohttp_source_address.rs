// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RG-7 — the application relay observes the OHTTP relay's address, never the
//! end user's (ADR-037).
//!
//! The client connects from 127.0.0.2 and the OHTTP relay forwards from
//! 127.0.0.1, so the two sources are distinguishable at the gateway. Linux
//! routes all of 127.0.0.0/8 to loopback; macOS configures only 127.0.0.1, so
//! the test is Linux-only (the CI runners are Linux).

#![cfg(target_os = "linux")]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::{Router, extract::ConnectInfo, http::StatusCode, routing::post};
use tokio::sync::Mutex;
use vauchi_e2e_tests::prelude::*;

const END_USER: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
const OHTTP_RELAY: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

/// A stand-in application relay that records each connection's source IP.
async fn observing_gateway() -> (String, Arc<Mutex<Vec<IpAddr>>>) {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    let app = Router::new().route(
        "/v2/ohttp",
        post(move |ConnectInfo(peer): ConnectInfo<SocketAddr>| {
            let sink = sink.clone();
            async move {
                sink.lock().await.push(peer.ip());
                StatusCode::BAD_REQUEST
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind observing gateway");
    let url = format!("http://{}", listener.local_addr().expect("gateway addr"));
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("observing gateway");
    });
    (url, observed)
}

fn end_user_client() -> reqwest::Client {
    reqwest::Client::builder()
        .local_address(END_USER)
        .build()
        .expect("client bound to the end-user address")
}

// @scenario: release_privacy_multidevice_certification.feature:Application relay sees the gateway under a distinct operator
// @internal
#[tokio::test]
async fn application_relay_observes_the_ohttp_relay_never_the_end_user() {
    let (gateway_url, observed) = observing_gateway().await;
    let mut ohttp = OhttpRelayManager::new(OhttpRelayConfig::default()).expect("ohttp manager");
    ohttp.spawn(&gateway_url).await.expect("spawn ohttp-relay");
    let ohttp_url = ohttp.url().expect("ohttp relay url");
    let client = end_user_client();

    // Positive control: a direct request shows the gateway can see 127.0.0.2.
    client
        .post(format!("{gateway_url}/v2/ohttp"))
        .body(vec![0x00])
        .send()
        .await
        .expect("direct request to the gateway");
    let direct = observed.lock().await.drain(..).collect::<Vec<_>>();

    for _ in 0..3 {
        client
            .post(format!("{ohttp_url}/v2/ohttp"))
            .header("Content-Type", "message/ohttp-req")
            .body(vec![0x01, 0x02, 0x03])
            .send()
            .await
            .expect("request through the OHTTP relay");
    }
    let relayed = observed.lock().await.clone();

    assert_eq!(
        direct,
        vec![END_USER],
        "the observer must be able to see the end-user address"
    );
    assert_eq!(
        relayed,
        vec![OHTTP_RELAY; 3],
        "every relayed request must arrive from the OHTTP relay, never from {END_USER}"
    );
}
