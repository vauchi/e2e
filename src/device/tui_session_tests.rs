// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use super::*;

async fn terminal_fixture(hold_selection: bool) -> TuiDevice {
    let data_dir = TempDir::new().unwrap();
    let program = data_dir.path().join("navigation.sh");
    std::fs::write(
        &program,
        r#"#!/bin/bash
stty -echo -icanon
status=Synced
sync_requested=no
printf '\033[2J\033[HContacts\033[2;1HAdd Contact\033[3;1HSynced'
while IFS= read -rsn1 key; do
    case "$key" in
    m)
        if [ "$sync_requested" = yes ]; then status='Sync failed'; fi
        printf '\033[2J\033[HContacts\033[2;1H%s\033[4;1H╔Navigate════╗\033[5;1H║2. Contacts║\033[6;1H╚═══════════╝' "$status"
        ;;
    s) sync_requested=yes ;;
    2)
        if [ "$HOLD_SELECTION" = yes ]; then continue; fi
        printf '\033[2J\033[HContacts\033[2;1HAdd Contact\033[3;1H%s' "$status"
        ;;
    '!') printf '\033[2J\033[HContacts\033[2;1HAdd Contact\033[3;1H%s' "$status" ;;
    q) exit 0 ;;
    esac
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let session = TuiSession::new(
        program.clone(),
        data_dir.path().to_path_buf(),
        "unused".into(),
        HashMap::new(),
    );
    let env = HashMap::from([(
        "HOLD_SELECTION".into(),
        if hold_selection { "yes" } else { "no" }.into(),
    )]);
    *session.pty.lock().await =
        Some(PtySession::new(&program, data_dir.path(), "unused", &env).unwrap());
    *session.is_running.lock().await = true;
    session.expect("Synced").await.unwrap();
    TuiDevice {
        name: "Fixture".into(),
        data_dir,
        relay_url: "unused".into(),
        session,
    }
}

// @internal
#[tokio::test]
async fn screen_snapshot_waits_for_contacts_overlay_to_close() {
    let device = Arc::new(terminal_fixture(true).await);
    let selected = device.clone();
    let mut navigation = tokio::spawn(async move { selected.session.navigate_to_contacts().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(800), &mut navigation)
            .await
            .is_err(),
        "the navigation menu's Contacts label is not the selected screen"
    );
    device.session.send_char('!').await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), navigation)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    device.session.stop().await.unwrap();
}

// @internal
#[tokio::test]
async fn screen_snapshot_sync_rejects_the_previous_success() {
    let device = terminal_fixture(false).await;
    let outcome = device.sync().await;
    assert!(
        outcome.is_err(),
        "the new action failed after an old success"
    );
    assert!(
        outcome
            .unwrap_err()
            .to_string()
            .contains("sync chip reports failure")
    );
    device.session.stop().await.unwrap();
}
