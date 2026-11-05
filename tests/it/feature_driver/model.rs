// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use vauchi_core::{ActionSpec, Command, Event, InputValue, PresentationNode, SurfaceId};

#[derive(Default)]
pub(super) struct PresentationModel {
    current: Vec<Command>,
}

impl PresentationModel {
    pub(super) fn apply(&mut self, batch: &[Command]) {
        for command in batch {
            match command {
                Command::ReplaceSurface { surface } => {
                    if self.current.iter().any(|command| {
                        matches!(command, Command::ReplaceSurface { surface: current }
                            if current.surface_id == surface.surface_id
                                && current.revision > surface.revision)
                    }) {
                        continue;
                    }
                    self.current.clear();
                    self.current.push(command.clone());
                }
                Command::SetContextBar {
                    surface_id,
                    revision,
                    ..
                } if self.is_current_revision(surface_id, *revision) => {
                    self.current
                        .retain(|old| !matches!(old, Command::SetContextBar { .. }));
                    self.current.push(command.clone());
                }
                Command::PresentOverlay {
                    surface_id,
                    revision,
                    ..
                } if self.is_current_revision(surface_id, *revision) => {
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
                _ => {}
            }
        }
    }

    fn is_current_revision(&self, id: &SurfaceId, revision: u64) -> bool {
        self.current.iter().any(|command| {
            matches!(command, Command::ReplaceSurface { surface }
                if &surface.surface_id == id && surface.revision == revision)
        })
    }

    pub(super) fn action(&self, label: &str) -> Option<Event> {
        self.current.iter().find_map(|command| match command {
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

    pub(super) fn navigation(&self) -> Option<Event> {
        self.current.iter().find_map(|command| match command {
            Command::SetContextBar {
                surface_id, bar, ..
            } => bar
                .navigation
                .as_ref()
                .map(|action| activate(surface_id, action)),
            _ => None,
        })
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
