// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Result, bail};
use std::sync::Arc;
use std::time::Duration;
use vauchi_app::ui::AppEngine;
use vauchi_core::clock::FakeClock;
use vauchi_core::{ActionSpec, Command, Event, InputValue, PresentationNode, SurfaceId};

pub(super) struct Actor {
    engine: AppEngine,
    pub(super) clock: Arc<FakeClock>,
    current: Vec<Command>,
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
            current: Vec::new(),
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
        for command in &batch {
            match command {
                Command::ReplaceSurface { surface } => {
                    self.current.retain(|old| match old {
                        Command::ReplaceSurface { .. } => false,
                        Command::SetContextBar { surface_id, .. } => {
                            *surface_id == surface.surface_id
                        }
                        _ => true,
                    });
                    self.current.push(command.clone());
                }
                Command::SetContextBar { .. } => {
                    self.current
                        .retain(|old| !matches!(old, Command::SetContextBar { .. }));
                    self.current.push(command.clone());
                }
                Command::PresentOverlay { .. } => {
                    self.current
                        .retain(|old| !matches!(old, Command::PresentOverlay { .. }));
                    self.current.push(command.clone());
                }
                Command::DismissOverlay {
                    surface_id, kind, ..
                } => {
                    self.current.retain(|old| {
                        !matches!(old,
                        Command::PresentOverlay { surface_id: old_surface, overlay, .. }
                        if old_surface == surface_id && overlay.kind == *kind)
                    });
                }
                Command::ScheduleWakeup { delay_millis, .. } => {
                    self.pending_wakeup = Some(*delay_millis)
                }
                _ => {}
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
        if let Some(event) = action(&self.current, label) {
            return self.dispatch(event);
        }
        let navigation = self.current.iter().find_map(|command| match command {
            Command::SetContextBar {
                surface_id, bar, ..
            } => bar
                .navigation
                .as_ref()
                .map(|action| activate(surface_id, action)),
            _ => None,
        });
        if let Some(event) = navigation {
            self.dispatch(event)?;
            if let Some(event) = action(&self.current, label) {
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
        fn rows(nodes: &[PresentationNode], names: &mut Vec<String>) {
            for node in nodes {
                match node {
                    PresentationNode::List { rows, .. } => {
                        names.extend(rows.iter().map(|row| row.title.clone()))
                    }
                    PresentationNode::Group { children, .. } => rows(children, names),
                    _ => {}
                }
            }
        }
        let mut names = Vec::new();
        for command in &self.current {
            if let Command::ReplaceSurface { surface } = command {
                rows(&surface.nodes, &mut names);
            }
        }
        names
    }
}

fn activate(surface: &SurfaceId, action: &ActionSpec) -> Event {
    Event::ActionActivated {
        surface_id: surface.clone(),
        interaction_id: action.interaction_id.clone(),
    }
}

fn node_action(surface: &SurfaceId, nodes: &[PresentationNode], label: &str) -> Option<Event> {
    for node in nodes {
        let event = match node {
            PresentationNode::Group { children, .. } => node_action(surface, children, label),
            PresentationNode::List { rows, .. } => rows.iter().find_map(|row| {
                if row.title == label && row.enabled {
                    row.activation
                        .as_ref()
                        .filter(|action| action.enabled)
                        .map(|action| activate(surface, action))
                } else {
                    node_action(surface, &row.controls, label)
                }
            }),
            PresentationNode::Choice {
                binding_id,
                options,
                enabled: true,
                ..
            } => options
                .iter()
                .find(|option| option.label == label)
                .map(|option| Event::ValueChanged {
                    surface_id: surface.clone(),
                    binding_id: binding_id.clone(),
                    value: InputValue::Choice(Some(option.id.clone())),
                }),
            _ => None,
        };
        if event.is_some() {
            return event;
        }
    }
    None
}

fn action(commands: &[Command], label: &str) -> Option<Event> {
    commands.iter().find_map(|command| match command {
        Command::SetContextBar {
            surface_id, bar, ..
        } => [&bar.back, &bar.navigation, &bar.primary, &bar.secondary]
            .into_iter()
            .filter_map(Option::as_ref)
            .find(|action| action.label == label && action.enabled)
            .map(|action| activate(surface_id, action)),
        Command::PresentOverlay {
            surface_id,
            overlay,
            ..
        } => overlay
            .items
            .iter()
            .find(|action| action.label == label && action.enabled)
            .map(|action| activate(surface_id, action)),
        Command::ReplaceSurface { surface } => {
            node_action(&surface.surface_id, &surface.nodes, label)
        }
        _ => None,
    })
}
