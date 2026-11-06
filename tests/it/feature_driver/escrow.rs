// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Instant;
use tokio::sync::oneshot;
use vauchi_relay::escrow::EscrowStore;

struct Store {
    escrow: EscrowStore,
    instant: Instant,
    delivery: AtomicBool,
    requests: AtomicUsize,
}

pub(super) struct EscrowAdapter {
    pub(super) url: String,
    state: Arc<Store>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl EscrowAdapter {
    pub(super) fn start() -> Result<Self> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(Store {
            escrow: EscrowStore::new(8),
            instant: Instant::now(),
            delivery: AtomicBool::new(true),
            requests: AtomicUsize::new(0),
        });
        let router = Router::new()
            .route("/v2/escrow", post(exchange))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(state.clone());
        let (shutdown, stopped) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("adapter runtime")
                .block_on(async move {
                    let listener =
                        tokio::net::TcpListener::from_std(listener).expect("adapter listener");
                    axum::serve(listener, router)
                        .with_graceful_shutdown(async {
                            let _ = stopped.await;
                        })
                        .await
                        .expect("escrow adapter server");
                });
        });
        Ok(Self {
            url,
            state,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    pub(super) fn block_delivery(&self) {
        self.state.delivery.store(false, Ordering::Relaxed);
    }

    pub(super) fn requests(&self) -> usize {
        self.state.requests.load(Ordering::Relaxed)
    }
}

impl Drop for EscrowAdapter {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("adapter shutdown");
        }
    }
}

async fn exchange(
    State(store): State<Arc<Store>>,
    Json(mut value): Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    store.requests.fetch_add(1, Ordering::Relaxed);
    if !store.delivery.load(Ordering::Relaxed) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let object = value.as_object_mut().ok_or(StatusCode::BAD_REQUEST)?;
    let action = object
        .remove("escrow_action")
        .ok_or(StatusCode::BAD_REQUEST)?;
    object.insert("action".into(), action);
    let message = serde_json::from_value(value).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(store.escrow.handle_at(message, store.instant)))
}
