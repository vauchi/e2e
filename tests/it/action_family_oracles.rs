// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Action-Family OHTTP Oracles (RG-8 follow-up)
//!
//! CLI-level oracles for the action families the fail-closed matrix
//! (`ohttp_fail_closed_matrix.rs`, sync path) does not cover — problem
//! record `2026-07-15-ohttp-dynamic-direct-fallback-regression`:
//!
//! 1. Device link: the full `link` → `join` → `complete` → `finish`
//!    chain passes request/response material out of band (QR + manual
//!    copy). Pin that every step completes against hostile plaintext
//!    relay endpoints while making ZERO requests to either hop — any
//!    direct dial is a fail-open regression.
//! 2. Panic shred: emergency erasure must complete locally even when
//!    the relay endpoints are hostile, and must not dial either hop
//!    when there is nothing to notify.
//!
//! Claim boundary: the core-level recorder tests of core!1393/1396 pin
//! the transport construction for these families; these oracles pin
//! the externally observable CLI behavior. A panic-shred-with-contacts
//! notification oracle (CLI-driven split-relay exchange to stage a
//! contact, then asserting the notification traverses only the outer
//! hop) is tracked as a follow-up in the same problem record.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use super::ohttp_fail_closed_matrix::{
    CLI_TIMEOUT, CliOutcome, HostileServer, KeyMode, cli_binary, init_probe_identity, run_cli,
};

/// Runs the CLI under a pty via script(1) so interactive confirmations
/// (e.g. "Type 'PANIC'") work. Exit status is not portable across
/// script(1) variants — assert on output markers, not `success`.
fn run_cli_pty(args: &[&str], stdin_text: &str, data_dir: &Path) -> CliOutcome {
    let bin = cli_binary();
    let mut full: Vec<String> = vec!["--data-dir".to_string(), data_dir.display().to_string()];
    full.extend(args.iter().map(|arg| arg.to_string()));

    let (program, pty_args): (&str, Vec<String>) = if cfg!(target_os = "macos") {
        let mut a = vec!["-q".to_string(), "/dev/null".to_string()];
        a.push(bin.display().to_string());
        a.extend(full.iter().cloned());
        ("script", a)
    } else {
        let line = std::iter::once(bin.display().to_string())
            .chain(full.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        (
            "script",
            vec!["-qec".to_string(), line, "/dev/null".to_string()],
        )
    };

    let mut child = Command::new(program)
        .args(&pty_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The harness-wide rule: the retired escape hatch must never
        // leak in from the parent environment.
        .env_remove("VAUCHI_ALLOW_DIRECT")
        .spawn()
        .expect("pty wrapper should spawn");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_text.as_bytes());
    }

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

/// First long base64-ish token after `marker` in CLI output.
fn scrape_token(output: &str, marker: &str) -> String {
    let Some((_, tail)) = output.split_once(marker) else {
        panic!("marker {marker:?} not found in output: {output}");
    };
    for token in tail.split_whitespace() {
        let trimmed = token.trim();
        if trimmed.len() >= 40
            && trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        {
            return trimmed.to_string();
        }
    }
    panic!("no base64 token after marker {marker:?} in output: {output}");
}

fn device_link_qr(data_dir: &Path, relay: &str, ohttp: &str) -> String {
    let outcome = run_cli(
        &[
            "device",
            "link",
            "--raw",
            "--relay",
            relay,
            "--ohttp-relay",
            ohttp,
        ],
        &[],
        data_dir,
    );
    assert!(
        outcome.success,
        "device link should succeed: {}",
        outcome.stderr
    );
    scrape_token(&outcome.stdout, "Device link data")
}

fn device_join_request(qr: &str, data_dir: &Path, relay: &str, ohttp: &str) -> String {
    let outcome = run_cli(
        &[
            "device",
            "join",
            qr,
            "--device-name",
            "B2",
            "-y",
            "--relay",
            relay,
            "--ohttp-relay",
            ohttp,
        ],
        &[],
        data_dir,
    );
    assert!(
        outcome.success,
        "device join should succeed: {}",
        outcome.stderr
    );
    scrape_token(&outcome.stdout, "Send this request")
}

// @scenario: release_privacy_multidevice_certification.feature:Neither relay can decrypt or identify application users
/// The whole device-link chain completes against hostile plaintext
/// endpoints while recording ZERO requests on either hop.
// @internal
#[test]
fn device_link_chain_makes_zero_requests_to_either_hop() {
    let app = HostileServer::start(KeyMode::Garbage);
    let outer = HostileServer::start(KeyMode::Garbage);

    let d1 = tempfile::tempdir().expect("initiator data dir");
    init_probe_identity(d1.path());
    let d2 = tempfile::tempdir().expect("joiner data dir (pre-identity)");

    let qr = device_link_qr(d1.path(), &app.url(), &outer.url());
    let request = device_join_request(&qr, d2.path(), &app.url(), &outer.url());

    let complete = run_cli(
        &[
            "device",
            "complete",
            &request,
            "-y",
            "--relay",
            &app.url(),
            "--ohttp-relay",
            &outer.url(),
        ],
        &[],
        d1.path(),
    );
    assert!(
        complete.success,
        "device complete should succeed: {}",
        complete.stderr
    );
    let response = scrape_token(&complete.stdout, "vauchi device finish ");

    let finish = run_cli(
        &[
            "device",
            "finish",
            &response,
            "--relay",
            &app.url(),
            "--ohttp-relay",
            &outer.url(),
        ],
        &[],
        d2.path(),
    );
    assert!(
        finish.success,
        "device finish should succeed: {}",
        finish.stderr
    );

    assert!(
        app.recorded().is_empty(),
        "device link must not dial the application relay: {:?}",
        app.recorded()
    );
    assert!(
        outer.recorded().is_empty(),
        "device link must not dial the OHTTP outer relay: {:?}",
        outer.recorded()
    );
}

// @scenario: release_privacy_multidevice_certification.feature:Neither relay can decrypt or identify application users
/// Panic shred completes local erasure against hostile endpoints and
/// dials the application relay never. The outer relay may see only the
/// designed purge leg as an opaque OHTTP-encapsulated POST — any other
/// request line (direct API path, key fetch against the wrong origin,
/// plaintext action) is a fail-open regression.
// @internal
#[test]
fn panic_shred_completes_local_erasure_without_network() {
    let app = HostileServer::start(KeyMode::Garbage);
    let outer = HostileServer::start(KeyMode::Garbage);

    let data_dir = tempfile::tempdir().expect("probe data dir");
    init_probe_identity(data_dir.path());

    let outcome = run_cli_pty(
        &[
            "gdpr",
            "panic-shred",
            "--relay",
            &app.url(),
            "--ohttp-relay",
            &outer.url(),
        ],
        "PANIC\n",
        data_dir.path(),
    );
    assert!(
        !outcome.timed_out,
        "panic shred must complete bounded, not hang; stdout: {}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains("Shred Report"),
        "panic shred should report completion, got stdout: {}",
        outcome.stdout
    );

    // Local erasure is the guarantee: the identity must be gone even
    // though every relay endpoint was hostile.
    let after = run_cli(&["contacts", "list"], &[], data_dir.path());
    assert!(
        !after.success && after.stderr.contains("not initialized"),
        "panic shred must erase the local identity, got: {}",
        after.stderr
    );

    assert!(
        app.recorded().is_empty(),
        "panic shred must not dial the application relay: {:?}",
        app.recorded()
    );
    // The shred's purge leg is allowed to contact the outer hop — but
    // only as an opaque OHTTP-encapsulated POST. Every other request
    // line is a leak.
    let outer_dials = outer.recorded();
    assert!(
        outer_dials
            .iter()
            .all(|line| line.starts_with("POST /v2/ohttp ")),
        "panic shred may contact the outer relay only via the encapsulated OHTTP path: {outer_dials:?}"
    );
}

// ── Shred fan-out oracles (with staged contacts) ───────────────────

use crate::ohttp_helpers::spawn_ohttp_stack;

/// Initialize a named identity against explicit endpoints.
fn init_named_identity(data_dir: &Path, name: &str, relay: &str, ohttp: &str) {
    let outcome = run_cli(
        &["init", name, "--relay", relay, "--ohttp-relay", ohttp],
        &[],
        data_dir,
    );
    assert!(
        outcome.success,
        "init {name} should succeed: {}",
        outcome.stderr
    );
}

/// First long base64-ish token in CLI output (QR payload line).
fn qr_token(output: &str) -> String {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.len() >= 40
            && !trimmed.contains(['█', '▀', '▄'])
            && trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        {
            return trimmed.to_string();
        }
    }
    panic!("no QR token in output: {output}");
}

fn exchange_start(data_dir: &Path, relay: &str, ohttp: &str) -> String {
    let outcome = run_cli(
        &[
            "exchange",
            "start",
            "--relay",
            relay,
            "--ohttp-relay",
            ohttp,
        ],
        &[],
        data_dir,
    );
    assert!(
        outcome.success,
        "exchange start should succeed: {}",
        outcome.stderr
    );
    qr_token(&outcome.stdout)
}

fn exchange_complete(data_dir: &Path, qr: &str, relay: &str, ohttp: &str) {
    let outcome = run_cli(
        &[
            "exchange",
            "complete",
            qr,
            "--relay",
            relay,
            "--ohttp-relay",
            ohttp,
        ],
        &[],
        data_dir,
    );
    assert!(
        outcome.success,
        "exchange complete should succeed: {}",
        outcome.stderr
    );
}

fn sync_cli(data_dir: &Path, relay: &str, ohttp: &str) -> CliOutcome {
    let outcome = run_cli(
        &["sync", "--relay", relay, "--ohttp-relay", ohttp],
        &[],
        data_dir,
    );
    assert!(outcome.success, "sync should succeed: {}", outcome.stderr);
    outcome
}

fn contacts_list(data_dir: &Path, relay: &str, ohttp: &str) -> String {
    let outcome = run_cli(
        &["contacts", "list", "--relay", relay, "--ohttp-relay", ohttp],
        &[],
        data_dir,
    );
    assert!(
        outcome.success,
        "contacts list should succeed: {}",
        outcome.stderr
    );
    outcome.stdout
}

// @scenario: release_privacy_multidevice_certification.feature:Neither relay can decrypt or identify application users
/// Panic shred WITH a staged contact: the per-contact revocation
/// delivery and the relay purge must traverse the OHTTP outer hop
/// only. The application-relay endpoint the CLI is configured with is
/// a hostile recorder — any direct dial (connect, purge, delivery)
/// trips the oracle. Bob must observe the revocation: his sync
/// processes the delivered blob and Alice disappears from his
/// contacts.
// @internal
#[tokio::test]
async fn panic_shred_with_contacts_fanout_over_outer_hop_only() {
    let (mut relay_mgr, mut ohttp_mgr, _relay_http_url, ohttp_url) = spawn_ohttp_stack().await;
    let app = HostileServer::start(KeyMode::Garbage);

    let alice = tempfile::tempdir().expect("alice data dir");
    let bob = tempfile::tempdir().expect("bob data dir");
    init_named_identity(alice.path(), "Alice", &app.url(), &ohttp_url);
    init_named_identity(bob.path(), "Bob", &app.url(), &ohttp_url);

    // Stage the contact through the OHTTP path: both sides start, both
    // complete, both sync — exactly the harness's mutual-exchange flow.
    let alice_qr = exchange_start(alice.path(), &app.url(), &ohttp_url);
    let bob_qr = exchange_start(bob.path(), &app.url(), &ohttp_url);
    exchange_complete(bob.path(), &alice_qr, &app.url(), &ohttp_url);
    exchange_complete(alice.path(), &bob_qr, &app.url(), &ohttp_url);
    sync_cli(alice.path(), &app.url(), &ohttp_url);
    sync_cli(bob.path(), &app.url(), &ohttp_url);
    assert!(
        contacts_list(alice.path(), &app.url(), &ohttp_url).contains("Bob"),
        "Alice should have Bob staged as a contact"
    );

    // Alice panic-shreds. The fan-out (relay purge + Bob's revocation
    // delivery) must go through the outer hop; the report must show
    // both legs delivered.
    let outcome = run_cli_pty(
        &[
            "gdpr",
            "panic-shred",
            "--relay",
            &app.url(),
            "--ohttp-relay",
            &ohttp_url,
        ],
        "PANIC\n",
        alice.path(),
    );
    assert!(
        !outcome.timed_out,
        "panic shred must complete bounded, not hang; stdout: {}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains("Shred Report"),
        "panic shred should report completion, got stdout: {}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains("Relay purge sent:       true"),
        "relay purge must be sent through the OHTTP path, got stdout: {}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains("Contacts notified:      1"),
        "Bob's revocation delivery must be sent through the OHTTP path, got stdout: {}",
        outcome.stdout
    );

    // Bob syncs: the delivered revocation crypto-shreds Alice's CEK and
    // removes her contact row.
    sync_cli(bob.path(), &app.url(), &ohttp_url);
    let bob_contacts = contacts_list(bob.path(), &app.url(), &ohttp_url);
    assert!(
        !bob_contacts.contains("Alice"),
        "Bob must observe Alice's revocation (contact removed), got: {bob_contacts}"
    );

    // Privacy leg: nothing may have dialed the application-relay
    // endpoint the devices were configured with.
    assert!(
        app.recorded().is_empty(),
        "no action may dial the application relay directly: {:?}",
        app.recorded()
    );

    ohttp_mgr.stop().await;
    relay_mgr.stop_all().await;
}
