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
    terminal.session.expect(expectrl::Eof).unwrap();
    assert!(
        terminal.read_screen_snapshot().is_err(),
        "a dead child cannot supply a successful retained screen"
    );
}
