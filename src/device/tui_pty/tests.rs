// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;

use super::PtySession;

// @internal
#[test]
fn screen_snapshot_retains_unchanged_cells_and_applies_screen_clears() {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("incremental-terminal.sh");
    std::fs::write(
        &program,
        r#"#!/bin/bash
stty -echo
printf '\033[2J\033[HContacts\033[2;1HAdd Contact\033[4;1HAlice'
while :; do
    if IFS= read -r -t 1 command; then
        case "$command" in
        update) printf '\033[4;1HBob  ' ;;
        clear) printf '\033[2J\033[HSettings' ;;
        *) exit 0 ;;
        esac
    else
        printf '\033[?25l'
    fi
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminal =
        PtySession::new(&program, directory.path(), "unused", &HashMap::new()).unwrap();
    terminal.expect("Alice").unwrap();
    terminal.send_text("update\n").unwrap();
    terminal.expect("Bob").unwrap();
    let updated = terminal.read_screen_snapshot().unwrap();
    assert!(
        updated.contains("Contacts"),
        "unchanged title is still visible: {updated:?}"
    );
    assert!(updated.contains("Bob"), "new row is visible: {updated:?}");
    assert!(
        !updated.contains("Alice"),
        "old row was overwritten: {updated:?}"
    );

    terminal.send_text("clear\n").unwrap();
    terminal.expect("Settings").unwrap();
    let cleared = terminal.read_screen_snapshot().unwrap();
    assert!(cleared.contains("Settings"));
    assert!(
        !cleared.contains("Contacts"),
        "cleared screens cannot satisfy a stale wait"
    );
    terminal.quit().unwrap();
}

// @internal
#[test]
fn screen_snapshot_rejects_a_terminated_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("terminated-terminal.sh");
    std::fs::write(&program, "#!/bin/bash\nprintf 'Contacts Synced'\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminal =
        PtySession::new(&program, directory.path(), "unused", &HashMap::new()).unwrap();
    terminal.expect("Synced").unwrap();
    let error = terminal.expect("unreachable marker").unwrap_err();
    assert!(error.to_string().contains("not found"));
    assert!(
        terminal.read_screen_snapshot().is_err(),
        "a dead child cannot supply a successful retained screen"
    );
}

// @internal
#[test]
fn terminal_environment_values_are_literal() {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("literal environment.sh");
    std::fs::write(
        &program,
        "#!/bin/bash\nstty -echo\nprintf '%s\\n' \"$MARKER\"\nread -r command\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let marker = r#"value "$LITERAL_UNDEFINED" $(printf expanded) 'quote' END"#;
    let mut terminal = PtySession::new(
        &program,
        directory.path(),
        "unused",
        &HashMap::from([("MARKER".into(), marker.into())]),
    )
    .unwrap();
    assert_eq!(terminal.expect("value .* END").unwrap(), marker);
}

// @internal
#[test]
fn terminal_matches_across_read_chunks_and_retains_the_unmatched_suffix() {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("chunked-terminal.sh");
    std::fs::write(
        &program,
        "#!/bin/bash\nstty -echo\nprintf 'START%5000sEND tail\\n' ''\nread -r command\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminal =
        PtySession::new(&program, directory.path(), "unused", &HashMap::new()).unwrap();
    assert_eq!(
        terminal.expect("START {5000}END").unwrap(),
        format!("START{}END", " ".repeat(5000))
    );
    assert_eq!(terminal.expect("tail").unwrap(), "tail");
    let error = terminal
        .expect_with_timeout("missing", std::time::Duration::ZERO)
        .unwrap_err();
    assert!(error.to_string().contains("not found"));
    terminal.send_text("complete\n").unwrap();
}

// @internal
#[test]
fn dropping_a_terminal_terminates_and_reaps_its_child() {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("child-lifetime.sh");
    std::fs::write(
        &program,
        "#!/bin/bash\nstty -echo\nprintf 'PID=%s\\n' \"$$\"\nread -r command\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminal =
        PtySession::new(&program, directory.path(), "unused", &HashMap::new()).unwrap();
    let pid = terminal.expect("PID=[0-9]+").unwrap();
    let pid = pid.strip_prefix("PID=").unwrap();
    drop(terminal);
    let status = std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1), "terminal child must be reaped");
}

// @internal
#[test]
fn terminal_rejects_excessive_unmatched_output() {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("excessive-output.sh");
    std::fs::write(
        &program,
        "#!/bin/bash\nstty -echo\nprintf '%1048577s' ''\nread -r command\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminal =
        PtySession::new(&program, directory.path(), "unused", &HashMap::new()).unwrap();
    let error = terminal.expect("never emitted").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unmatched output exceeded 1 MiB"),
        "{error}"
    );
}
