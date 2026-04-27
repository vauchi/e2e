// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tracing-flame integration for span-aware profiling of e2e flows.
//!
//! Enabled via the `flame` cargo feature. Captures `tracing` spans
//! emitted in-process (e.g. by `vauchi_core`, `vauchi_app`, and the
//! e2e orchestrator). Spans from subprocess CLIs/relays launched by
//! the orchestrator are not captured — those are separate processes.
//!
//! Output: a `.folded` file written under `e2e/artifacts/flame/` (or
//! the path in `VAUCHI_FLAME_OUT`). Render to SVG with `inferno-flamegraph`
//! or via `just flame-render`.
//!
//! Span filter: defaults to `info,vauchi_core=trace,vauchi_app=trace`,
//! overridable via `VAUCHI_FLAME_FILTER` (env-filter syntax).

use std::fs::File;
use std::path::PathBuf;

use tracing_flame::FlameLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_FILTER: &str = "info,vauchi_core=trace,vauchi_app=trace,vauchi_e2e_tests=trace";

/// Initialise the global tracing subscriber with a flame layer.
///
/// Writes are unbuffered — each span enter/exit syncs to the file. This is
/// slower than the default [`FlameLayer::with_file`] (which wraps in a
/// [`BufWriter`]), but ensures data lands on disk even when libtest calls
/// `process::exit()` and skips static destructors.
pub fn init_layer() {
    let path = output_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("flame: create {} failed: {e}", parent.display()));
    }

    let file = File::create(&path)
        .unwrap_or_else(|e| panic!("flame: open {} failed: {e}", path.display()));
    let flame_layer = FlameLayer::new(file);

    let filter = std::env::var("VAUCHI_FLAME_FILTER")
        .ok()
        .and_then(|s| EnvFilter::try_new(s).ok())
        .unwrap_or_else(|| EnvFilter::new(DEFAULT_FILTER));

    let result = tracing_subscriber::registry()
        .with(filter)
        .with(flame_layer)
        .try_init();

    match result {
        Ok(_) => eprintln!("[flame] writing folded trace -> {}", path.display()),
        Err(e) => eprintln!(
            "[flame] WARNING: subscriber install failed ({e}); spans may already be captured by another subscriber"
        ),
    }
}

fn output_path() -> PathBuf {
    if let Ok(p) = std::env::var("VAUCHI_FLAME_OUT") {
        return PathBuf::from(p);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("artifacts/flame")
        .join(format!("trace-{ts}.folded"))
}
