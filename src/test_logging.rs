// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tracing setup for e2e integration tests.
//!
//! Installs a global tracing subscriber once at process start. The
//! subscriber always carries a fmt layer so subprocess logs forwarded
//! by `relay_manager` / `ohttp_relay_manager` (via
//! `tracing::warn!(target = "relay", ...)` and friends) reach the test
//! output. When the `flame` feature is enabled, a tracing-flame layer
//! is composed on top to produce `.folded` profiles.
//!
//! # Why this module exists
//!
//! Before 2026-04-27 only the flame layer was installed (and only
//! under `--features flame`). That meant the orchestrator's
//! `warn!(target = "relay", ...)` statements were silently dropped —
//! when an e2e test failed because the relay returned an error, the
//! relay's own log line explaining the failure went nowhere, and
//! diagnosis required attaching a debugger or rebuilding with the
//! flame feature. See `_private/docs/problems/2026-04-27-e2e-sync-http-400/`.
//!
//! # Filtering
//!
//! - `RUST_LOG` (env-filter syntax) controls the fmt layer. When
//!   unset, the default is `warn` — happy-path runs stay silent.
//!   Subprocess output still surfaces on failure because the
//!   orchestrator forwards each line as
//!   `tracing::warn!(target = "relay", ...)`, which is itself at
//!   WARN level and passes the default filter. Set `RUST_LOG=info`
//!   to also see orchestrator-level progress (user setup, device
//!   linking, etc.).
//! - `VAUCHI_FLAME_FILTER` controls the flame layer (only present
//!   under `--features flame`). Default:
//!   `info,vauchi_core=trace,vauchi_app=trace,vauchi_e2e_tests=trace`.
//!
//! # Output
//!
//! - Fmt: stderr (via libtest's test writer — captured per-test, shown
//!   on failure or under `--nocapture`).
//! - Flame (when enabled): `.folded` file at `$VAUCHI_FLAME_OUT` or
//!   `<CARGO_MANIFEST_DIR>/artifacts/flame/trace-<ts>.folded`.

use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_FMT_FILTER: &str = "warn";

#[cfg(feature = "flame")]
const DEFAULT_FLAME_FILTER: &str = "info,vauchi_core=trace,vauchi_app=trace,vauchi_e2e_tests=trace";

/// Install the global tracing subscriber. Idempotent — subsequent
/// calls are no-ops.
///
/// Always installs an fmt layer so subprocess stderr forwarded by the
/// orchestrator reaches the test output. With `--features flame`, also
/// installs a tracing-flame layer.
pub fn init() {
    let fmt_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FMT_FILTER));
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_test_writer()
        .with_target(true)
        .with_filter(fmt_filter);

    let registry = tracing_subscriber::registry().with(fmt_layer);

    #[cfg(feature = "flame")]
    {
        let path = flame_output_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("flame: create {} failed: {e}", parent.display()));
        }
        let file = std::fs::File::create(&path)
            .unwrap_or_else(|e| panic!("flame: open {} failed: {e}", path.display()));
        eprintln!("[flame] writing folded trace -> {}", path.display());

        let flame_filter = EnvFilter::try_new(
            std::env::var("VAUCHI_FLAME_FILTER")
                .unwrap_or_else(|_| DEFAULT_FLAME_FILTER.to_string()),
        )
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FLAME_FILTER));

        let flame_layer = tracing_flame::FlameLayer::new(file).with_filter(flame_filter);
        let _ = registry.with(flame_layer).try_init();
    }
    #[cfg(not(feature = "flame"))]
    {
        let _ = registry.try_init();
    }
}

#[cfg(feature = "flame")]
fn flame_output_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("VAUCHI_FLAME_OUT") {
        return std::path::PathBuf::from(p);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    base.join("artifacts/flame")
        .join(format!("trace-{ts}.folded"))
}
