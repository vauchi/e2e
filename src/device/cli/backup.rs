// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Explicit full and identity-only backup commands for CLI journeys.

use super::CliDevice;
use crate::error::{E2eError, E2eResult};

impl CliDevice {
    pub async fn export_full_backup(&self, password: &str) -> E2eResult<String> {
        self.export_backup_file(password, true).await
    }

    pub async fn import_full_backup(&self, path: &str, password: &str) -> E2eResult<()> {
        self.import_backup_file(path, password, true).await
    }

    pub(super) async fn export_backup_file(&self, password: &str, full: bool) -> E2eResult<String> {
        let path = self
            .data_dir
            .path()
            .join("backup.vauchi")
            .to_string_lossy()
            .to_string();
        let mut args = vec!["export", &path];
        if full {
            args.push("--full");
        }
        self.run_command_success_with_env(&args, &[("VAUCHI_BACKUP_PASSWORD", password)])
            .await?;
        Ok(path)
    }

    pub(super) async fn import_backup_file(
        &self,
        path: &str,
        password: &str,
        full: bool,
    ) -> E2eResult<()> {
        let mut args = vec!["import", path];
        if full {
            args.push("--full");
        }
        let output = self
            .run_command_success_with_env(&args, &[("VAUCHI_BACKUP_PASSWORD", password)])
            .await?;
        let public_id = output
            .lines()
            .find_map(|line| line.trim().strip_prefix("Public ID: "))
            .ok_or_else(|| E2eError::device("restored CLI did not report its public identity"))?;
        *self.public_id.lock().unwrap() = Some(public_id.trim().to_string());
        Ok(())
    }
}
