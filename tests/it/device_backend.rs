// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! E2E_DEVICE_BACKEND picks the backend for devices a test adds without
//! naming one: Core's merge gate runs the suite on the Core device, the
//! CLI's pipeline on the CLI (vauchi/private#1118).

use vauchi_e2e_tests::device::{DeviceType, default_device_type_from};

// @internal
#[test]
fn the_cli_stays_the_default_backend() {
    for value in [None, Some(""), Some("cli"), Some(" cli ")] {
        assert_eq!(
            default_device_type_from(value).unwrap(),
            DeviceType::Cli,
            "E2E_DEVICE_BACKEND={value:?} must keep the CLI"
        );
    }
}

// @internal
#[test]
fn core_selects_the_in_process_core_device() {
    assert_eq!(
        default_device_type_from(Some("core")).unwrap(),
        DeviceType::Core
    );
}

// @internal
#[test]
fn an_unknown_backend_is_refused_rather_than_run_on_the_cli() {
    for value in ["Core", "tui", "cli,core", "1"] {
        let error = default_device_type_from(Some(value)).unwrap_err();
        assert!(
            error.to_string().contains("expected `cli` or `core`"),
            "E2E_DEVICE_BACKEND={value} gave: {error}"
        );
    }
}
