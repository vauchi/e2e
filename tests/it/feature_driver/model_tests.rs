// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::model::PresentationModel;
use vauchi_core::{
    ActionSpec, ActionTone, Command, Event, InteractionId, OverlayKind, OverlaySpec, SurfaceId,
};

fn surface(id: &str, revision: u64) -> Command {
    Command::ReplaceSurface {
        surface: serde_json::from_value(serde_json::json!({
            "surface_id": id, "revision": revision, "title": "Fixture",
            "subtitle": null, "accessibility_label": "Fixture", "layout": "scroll",
            "nodes": [],
            "tokens": {
                "spacing_small": 1, "spacing_medium": 2, "spacing_large": 3,
                "corner_radius": 1, "minimum_target_size": 1,
                "colors": {
                    "background": "#000000", "surface": "#000000", "text": "#ffffff",
                    "text_muted": "#aaaaaa", "accent": "#ffffff", "border": "#ffffff",
                    "success": "#00ff00", "warning": "#ffff00", "error": "#ff0000"
                }
            }
        }))
        .expect("valid contract surface"),
    }
}

fn overlay(id: &str, revision: u64, kind: OverlayKind, label: &str) -> Command {
    Command::PresentOverlay {
        surface_id: SurfaceId::new(id).unwrap(),
        revision,
        overlay: OverlaySpec {
            kind,
            title: None,
            items: vec![ActionSpec {
                interaction_id: InteractionId::new(label).unwrap(),
                label: label.into(),
                accessibility_label: label.into(),
                icon_token: None,
                enabled: true,
                tone: ActionTone::Standard,
                shortcut: None,
            }],
            body: None,
            close_label: None,
        },
    }
}

fn activation(id: &str, label: &str) -> Option<Event> {
    Some(Event::ActionActivated {
        surface_id: SurfaceId::new(id).unwrap(),
        interaction_id: InteractionId::new(label).unwrap(),
    })
}

// @internal
#[test]
fn replacement_expires_overlay_actions_even_on_the_same_revision() {
    for (next_id, next_revision) in [("primary", 1), ("primary", 2), ("detail", 1)] {
        let mut model = PresentationModel::default();
        model.apply(&[
            surface("primary", 1),
            overlay("primary", 1, OverlayKind::Navigation, "old"),
        ]);
        assert_eq!(model.action("old"), activation("primary", "old"));
        model.apply(&[surface(next_id, next_revision)]);
        assert_eq!(
            model.action("old"),
            None,
            "replacement {next_id}/{next_revision}"
        );
    }
}

// @internal
#[test]
fn a_replacement_batch_can_present_a_new_overlay() {
    let mut model = PresentationModel::default();
    model.apply(&[
        surface("primary", 1),
        overlay("primary", 1, OverlayKind::Navigation, "old"),
    ]);
    model.apply(&[
        surface("primary", 2),
        overlay("primary", 2, OverlayKind::ActionMenu, "new"),
    ]);
    assert_eq!(model.action("old"), None);
    assert_eq!(model.action("new"), activation("primary", "new"));
}

// @internal
#[test]
fn an_older_surface_replacement_cannot_expire_the_current_overlay() {
    let mut model = PresentationModel::default();
    model.apply(&[
        surface("primary", 2),
        overlay("primary", 2, OverlayKind::Navigation, "current"),
    ]);
    model.apply(&[surface("primary", 1)]);
    assert_eq!(model.action("current"), activation("primary", "current"));
}

// @internal
#[test]
fn overlays_for_missing_or_noncurrent_surface_revisions_are_ignored() {
    for (id, revision) in [("missing", 2), ("primary", 1), ("primary", 3)] {
        let mut model = PresentationModel::default();
        model.apply(&[
            surface("primary", 2),
            overlay(id, revision, OverlayKind::Navigation, "stale"),
        ]);
        assert_eq!(model.action("stale"), None, "{id}/{revision}");
    }
}

// @internal
#[test]
fn overlay_dismissal_is_scoped_by_surface_and_kind_and_is_idempotent() {
    let mut model = PresentationModel::default();
    model.apply(&[
        surface("primary", 1),
        overlay("primary", 1, OverlayKind::Navigation, "open"),
    ]);
    for (id, kind) in [
        ("other", OverlayKind::Navigation),
        ("primary", OverlayKind::ActionMenu),
    ] {
        model.apply(&[Command::DismissOverlay {
            surface_id: SurfaceId::new(id).unwrap(),
            revision: 1,
            kind,
        }]);
        assert_eq!(model.action("open"), activation("primary", "open"));
    }
    let dismiss = Command::DismissOverlay {
        surface_id: SurfaceId::new("primary").unwrap(),
        revision: 1,
        kind: OverlayKind::Navigation,
    };
    model.apply(&[dismiss.clone(), dismiss]);
    assert_eq!(model.action("open"), None);
}
