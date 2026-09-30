// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! VAUCHI_E2E_NO_DEVICES keeps the suite off attached phones and booted
//! simulators (#396): a push from one session must not drive a device
//! another session is testing on.

use vauchi_e2e_tests::device::devices_allowed_from;

// @internal
#[test]
fn devices_are_used_unless_the_opt_out_is_set() {
    assert!(devices_allowed_from(None));
    assert!(devices_allowed_from(Some("")));
    assert!(devices_allowed_from(Some("0")));
}

// @internal
#[test]
fn any_other_value_of_the_opt_out_keeps_devices_untouched() {
    for value in ["1", "true", "yes", "on"] {
        assert!(
            !devices_allowed_from(Some(value)),
            "VAUCHI_E2E_NO_DEVICES={value} must keep devices untouched"
        );
    }
}
