// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::model::PresentationModel;
use anyhow::{Result, bail};
use std::sync::Arc;
use std::time::Duration;
use vauchi_app::ui::AppEngine;
use vauchi_core::clock::FakeClock;
use vauchi_core::{Command, Event};

pub(super) struct Actor {
    engine: AppEngine,
    pub(super) clock: Arc<FakeClock>,
    model: PresentationModel,
    pub(super) emitted: Vec<Command>,
    pending_wakeup: Option<u32>,
    pub(super) drop_presentation_updates: bool,
    pub(super) _storage: tempfile::TempDir,
}

impl Actor {
    pub(super) fn new(
        engine: AppEngine,
        clock: Arc<FakeClock>,
        storage: tempfile::TempDir,
    ) -> Result<Self> {
        let mut actor = Self {
            engine,
            clock,
            model: PresentationModel::default(),
            emitted: Vec::new(),
            pending_wakeup: None,
            drop_presentation_updates: false,
            _storage: storage,
        };
        let initial = actor.engine.initial_commands()?;
        actor.apply(initial);
        Ok(actor)
    }

    fn apply(&mut self, batch: Vec<Command>) {
        self.model.apply(&batch);
        for command in &batch {
            if let Command::ScheduleWakeup { delay_millis, .. } = command {
                self.pending_wakeup = Some(*delay_millis);
            }
        }
        self.emitted.extend(batch);
    }

    pub(super) fn dispatch(&mut self, event: Event) -> Result<()> {
        let mut batch = self.engine.dispatch(event)?;
        if self.drop_presentation_updates {
            batch.retain(|command| {
                !matches!(
                    command,
                    Command::ReplaceSurface { .. }
                        | Command::SetContextBar { .. }
                        | Command::PresentOverlay { .. }
                        | Command::DismissOverlay { .. }
                )
            });
        }
        self.apply(batch);
        Ok(())
    }

    pub(super) fn activate(&mut self, label: &str) -> Result<()> {
        if let Some(event) = self.model.action(label) {
            return self.dispatch(event);
        }
        let navigation = self.model.navigation();
        if let Some(event) = navigation {
            self.dispatch(event)?;
            if let Some(event) = self.model.action(label) {
                return self.dispatch(event);
            }
        }
        bail!("no presented enabled action named {label:?}")
    }

    pub(super) fn wake(&mut self) -> Result<()> {
        let delay = self
            .pending_wakeup
            .take()
            .ok_or_else(|| anyhow::anyhow!("Core did not schedule an exchange wakeup"))?;
        self.clock.advance(Duration::from_millis(u64::from(delay)));
        self.dispatch(Event::WakeupFired)
    }

    pub(super) fn celebrations(&self) -> usize {
        self.emitted
            .iter()
            .filter(|command| matches!(command, Command::Celebrate { .. }))
            .count()
    }

    pub(super) fn shared_url(&self) -> Result<String> {
        self.emitted
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::ShowShareSheet { url } => Some(url.clone()),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("Core did not request sharing the link"))
    }

    pub(super) fn contact_names(&self) -> Vec<String> {
        self.model.contact_names()
    }
}
