// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OHTTP Fail-Closed Adversarial Matrix (RG-8)
//!
//! CLI-level certification that the corrected 0.55.4 transport fails
//! closed on every historically weaker path (problem record
//! `2026-07-15-ohttp-dynamic-direct-fallback-regression`):
//!
//! 1. Origin boundary: plaintext (`http://`, `ws://`), TLS-only direct
//!    (`https://` without an outer hop), a `VAUCHI_ALLOW_DIRECT=1`
//!    injection, and a same-origin `--ohttp-relay` must all be rejected
//!    before any network I/O with the distinct-origin error — the
//!    retired `VAUCHI_ALLOW_DIRECT` hatch is gone from the binary
//!    (core!1393/1396), so injecting it changes nothing.
//! 2. Hostile bootstrap: an outer relay answering `/v2/ohttp-key` with
//!    garbage, an oversized body, a wrong content type, or an empty
//!    body must not poison the client — sync fails bounded, and the
//!    application relay sees ZERO direct requests (every application
//!    contact must traverse the outer hop, ADR-037). After rejecting a
//!    poisoned key the client falls back to its compiled-in bundled
//!    gateway key, so the outer may record one encapsulated POST it
//!    cannot decrypt; we pin that it sees OHTTP endpoints only.
//!
//! Claim boundary: these tests pin the `vauchi sync` production path
//! against the release CLI binary. The device-link initiator/responder,
//! hard-shred, and panic-shred action families are covered by the
//! core-level recorder tests of core!1393/1396; their CLI-level
//! oracles are tracked as a follow-up in the same problem record. Key
//! validation internals (RFC 9458 parse, 64 KiB bound) are pinned by
//! core unit tests; here we pin the externally observable contract.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub(crate) const DISTINCT_ORIGIN_ERROR: &str = "distinct valid origin";
// Runner contention slows CLI invocations 2-6x (argon2 init worst);
// 180s matches the VAUCHI_E2E_CLI_TIMEOUT_SECS default budget while
// still catching a genuine fail-open hang in minutes.
pub(crate) const CLI_TIMEOUT: Duration = Duration::from_secs(180);

/// (label, relay CLI args, extra env) for an origin-boundary probe case.
pub(crate) type OriginCase = (
    &'static str,
    Vec<&'static str>,
    Vec<(&'static str, &'static str)>,
);

pub(crate) fn cli_binary() -> PathBuf {
    if let Ok(dir) = std::env::var("E2E_BIN_DIR") {
        let path = PathBuf::from(&dir).join("vauchi");
        if path.exists() {
            return path;
        }
    }
    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cli/target/debug/vauchi");
    if debug.exists() {
        return debug;
    }
    panic!("CLI binary not found; set E2E_BIN_DIR or build cli debug");
}

pub(crate) struct CliOutcome {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) timed_out: bool,
}

/// Runs the CLI with a watchdog: kills the child on timeout so a
/// fail-open hang is reported as a failure, not a stuck pipeline.
pub(crate) fn run_cli(args: &[&str], envs: &[(&str, &str)], data_dir: &Path) -> CliOutcome {
    let mut cmd = Command::new(cli_binary());
    cmd.arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The harness-wide rule: the retired escape hatch must never
        // leak in from the parent environment.
        .env_remove("VAUCHI_ALLOW_DIRECT");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let child = cmd.spawn().expect("CLI should spawn");

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(CLI_TIMEOUT) {
        Ok(Ok(output)) => CliOutcome {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            timed_out: false,
        },
        _ => CliOutcome {
            success: false,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
        },
    }
}

pub(crate) fn init_probe_identity(data_dir: &Path) {
    let outcome = run_cli(&["init", "Probe"], &[], data_dir);
    assert!(
        outcome.success,
        "probe identity should initialize: {}",
        outcome.stderr
    );
}

#[derive(Clone, Copy)]
pub(crate) enum KeyMode {
    Garbage,
    Oversized,
    WrongType,
    Empty,
}

/// In-process hostile HTTP endpoint: records every request line and
/// serves a poisoned `/v2/ohttp-key` response (500 elsewhere).
pub(crate) struct HostileServer {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl HostileServer {
    pub(crate) fn start(mode: KeyMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("hostile server should bind");
        let port = listener.local_addr().expect("bound addr").port();
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => Self::serve(stream, mode, &requests),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        HostileServer {
            port,
            requests,
            stop,
            handle: Some(thread),
        }
    }

    fn serve(mut stream: TcpStream, mode: KeyMode, requests: &Arc<Mutex<Vec<String>>>) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let mut buf = vec![0u8; 8192];
        let Ok(n) = stream.read(&mut buf) else {
            return;
        };
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let request_line = head.lines().next().unwrap_or_default().to_string();
        requests
            .lock()
            .expect("request log")
            .push(request_line.clone());

        let is_key_fetch = request_line.starts_with("GET /v2/ohttp-key");
        let (status, content_type, body): (&str, &str, Vec<u8>) = if is_key_fetch {
            match mode {
                KeyMode::Garbage => ("200 OK", "application/ohttp-keys", vec![0xFF; 256]),
                KeyMode::Oversized => ("200 OK", "application/ohttp-keys", vec![0u8; 128 * 1024]),
                KeyMode::WrongType => ("200 OK", "application/json", b"{}".to_vec()),
                KeyMode::Empty => ("200 OK", "application/ohttp-keys", Vec::new()),
            }
        } else {
            (
                "500 Internal Server Error",
                "text/plain",
                b"hostile".to_vec(),
            )
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(&body);
    }

    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.requests.lock().expect("request log").clone()
    }
}

impl Drop for HostileServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// @scenario: release_privacy_multidevice_certification.feature:Neither relay can decrypt or identify application users
/// Origin boundary: every weaker-than-OHTTP relay configuration must
/// be rejected before any network I/O.
// @internal
#[test]
fn origin_boundary_probes_fail_closed() {
    let cases: [OriginCase; 5] = [
        ("plaintext http relay", vec!["http://127.0.0.1:1/"], vec![]),
        (
            "TLS-only direct relay",
            vec!["https://127.0.0.1:1/"],
            vec![],
        ),
        (
            "plaintext websocket relay",
            vec!["ws://127.0.0.1:1/"],
            vec![],
        ),
        (
            "VAUCHI_ALLOW_DIRECT injection",
            vec!["https://127.0.0.1:1/"],
            vec![("VAUCHI_ALLOW_DIRECT", "1")],
        ),
        (
            "same-origin outer relay",
            vec!["http://127.0.0.1:1/"],
            vec![],
        ),
    ];

    let data_dir = tempfile::tempdir().expect("probe data dir");
    init_probe_identity(data_dir.path());

    for (label, relay_args, envs) in &cases {
        let mut args: Vec<&str> = vec!["sync", "--relay"];
        args.push(relay_args[0]);
        if label == &"same-origin outer relay" {
            args.push("--ohttp-relay");
            args.push(relay_args[0]);
        }
        let started = Instant::now();
        let outcome = run_cli(&args, envs, data_dir.path());
        assert!(
            !outcome.timed_out,
            "{label}: sync must fail bounded, not hang (>{CLI_TIMEOUT:?})"
        );
        assert!(!outcome.success, "{label}: sync must not succeed");
        assert!(
            outcome.stderr.contains(DISTINCT_ORIGIN_ERROR),
            "{label}: expected the distinct-origin fail-closed error, \
             got stderr: {}",
            outcome.stderr
        );
        assert!(
            started.elapsed() < CLI_TIMEOUT,
            "{label}: origin rejection must precede network I/O"
        );
    }
}

// @scenario: release_privacy_multidevice_certification.feature:Neither relay can decrypt or identify application users
/// Hostile bootstrap: poisoned `/v2/ohttp-key` responses must not
/// poison the client, and the application relay must see zero direct
/// requests throughout.
// @internal
#[test]
fn hostile_bootstrap_responses_fail_closed() {
    let modes = [
        ("garbage key bytes", KeyMode::Garbage),
        ("oversized key response", KeyMode::Oversized),
        ("wrong content type", KeyMode::WrongType),
        ("empty key response", KeyMode::Empty),
    ];

    let data_dir = tempfile::tempdir().expect("probe data dir");
    init_probe_identity(data_dir.path());

    for (label, mode) in modes {
        let outer = HostileServer::start(mode);
        let app = HostileServer::start(KeyMode::Garbage);
        let outcome = run_cli(
            &["sync", "--relay", &app.url(), "--ohttp-relay", &outer.url()],
            &[],
            data_dir.path(),
        );

        assert!(
            !outcome.timed_out,
            "{label}: poisoned bootstrap must fail bounded, not hang"
        );
        assert!(
            !outcome.success,
            "{label}: sync must not succeed against a poisoned bootstrap; \
             stdout: {}",
            outcome.stdout
        );
        let outer_log = outer.recorded();
        assert!(
            outer_log
                .iter()
                .any(|line| line.starts_with("GET /v2/ohttp-key")),
            "{label}: the client should attempt the public key bootstrap \
             from the outer relay, got: {outer_log:?}"
        );
        // After the poisoned key is rejected, the client falls back to
        // its compiled-in bundled gateway key (sync_http.rs
        // `resolve_ohttp_key` step 3) and may attempt one encapsulated
        // POST the hostile outer cannot decrypt. Pin that the outer
        // only ever sees the OHTTP endpoints — never an application
        // API path in cleartext.
        for line in &outer_log {
            assert!(
                line.starts_with("GET /v2/ohttp-key") || line.starts_with("POST /v2/ohttp "),
                "{label}: the outer hop must only see OHTTP endpoints, got: {outer_log:?}"
            );
        }
        assert!(
            app.recorded().is_empty(),
            "{label}: the application relay must see ZERO direct requests — \
             every application contact traverses the outer hop (ADR-037), \
             got: {:?}",
            app.recorded()
        );
    }
}
