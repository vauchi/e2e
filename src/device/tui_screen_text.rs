// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Text the TUI prints that the harness reads back off the screen: the
//! exchange link, the device-link invitation and the confirmation code.

pub(super) const DEVICE_LINK_PREFIX: &str = "vauchi://device-link?";

/// The device-link invitation URL the terminal prints under its QR label.
///
/// The URL is longer than a terminal row, so the renderer hard-wraps it
/// inside the surface border: a row that runs up to the border continues
/// on the next one, and a row padded before the border ends the URL.
pub(super) fn device_link_invitation_in(screen: &str) -> Option<String> {
    let mut rows = screen
        .lines()
        .skip_while(|row| !row.contains(DEVICE_LINK_PREFIX));
    let first = rows.next()?;
    let mut segment = &first[first.find(DEVICE_LINK_PREFIX)?..];
    let mut link = String::new();
    loop {
        let inner = segment.strip_suffix('│').unwrap_or(segment);
        let piece: String = inner.chars().take_while(|c| !c.is_whitespace()).collect();
        let ended = piece.len() < inner.len();
        link.push_str(&piece);
        if ended {
            break;
        }
        let Some(next) = rows.next() else { break };
        segment = next.strip_prefix('│').unwrap_or(next);
    }
    link.contains("code=").then_some(link)
}

/// The `ddd-ddd` code both devices show during a device link.
pub(super) fn confirmation_code_in(screen: &str) -> Option<String> {
    screen
        .split(|c: char| !(c.is_ascii_digit() || c == '-'))
        .find(|token| {
            let bytes = token.as_bytes();
            bytes.len() == 7
                && bytes[3] == b'-'
                && bytes
                    .iter()
                    .enumerate()
                    .all(|(i, b)| i == 3 || b.is_ascii_digit())
        })
        .map(str::to_string)
}

/// The exchange link on a Link share screen: Core's
/// `https://vauchi.app/x#…` form since 0.81.0 (vauchi/private#277), or the
/// `vauchi://exchange?…` form older builds print.
pub(super) fn share_link_in(screen: &str) -> Option<String> {
    const LINK_PREFIXES: [&str; 2] = ["https://vauchi.app/x#", "vauchi://exchange?"];
    screen.split_whitespace().find_map(|token| {
        LINK_PREFIXES.iter().find_map(|prefix| {
            let start = token.find(prefix)?;
            let link = token[start..].trim_end_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '=' && c != '_' && c != '-'
            });
            link.contains("pk=").then(|| link.to_string())
        })
    })
}

// INLINE_TEST_REQUIRED: the parsers are private to the feature-gated tui
// module and not reachable from tests/.
#[cfg(test)]
mod tests {
    use super::*;

    // @internal
    /// Core 0.81.0 shares `https://vauchi.app/x#pk=…&n=…&nm=…`
    /// (vauchi/private#277); the harness must read that off the share
    /// screen as it read the old `vauchi://exchange?…` form.
    #[test]
    fn the_share_screen_link_is_read_in_its_https_form() {
        let screen = "Send this link:\n  https://vauchi.app/x#pk=AAAA_-&n=BBBB&nm=QWxpY2U.  \n";

        assert_eq!(
            share_link_in(screen).as_deref(),
            Some("https://vauchi.app/x#pk=AAAA_-&n=BBBB&nm=QWxpY2U")
        );
    }

    // @internal
    #[test]
    fn the_share_screen_link_is_still_read_in_its_legacy_form() {
        let screen = "Send this link: vauchi://exchange?pk=AAAA&n=BBBB)";

        assert_eq!(
            share_link_in(screen).as_deref(),
            Some("vauchi://exchange?pk=AAAA&n=BBBB")
        );
    }

    // @internal
    #[test]
    fn a_screen_without_an_exchange_link_yields_none() {
        assert_eq!(share_link_in("Visit https://vauchi.app/x for help"), None);
        assert_eq!(share_link_in("Exchange Mode — Pick a way"), None);
    }

    // @internal
    /// Rows as the TUI drew them on 2026-10-23: the invitation hard-wrapped
    /// inside the surface border, the second row padded before it.
    #[test]
    fn a_device_link_invitation_wrapped_inside_the_border_is_joined() {
        let screen = "│[QR] Scan on new device│\n\
                      │  vauchi://device-link?qr=AAAA│\n\
                      │BBBB&code=MDkx      │\n\
                      │Expires at 1792731197│";
        assert_eq!(
            device_link_invitation_in(screen).as_deref(),
            Some("vauchi://device-link?qr=AAAABBBB&code=MDkx")
        );
    }

    // @internal
    #[test]
    fn a_device_link_invitation_on_one_row_ends_at_its_padding() {
        let screen = "│  vauchi://device-link?qr=AAAA&code=MDkx   │";
        assert_eq!(
            device_link_invitation_in(screen).as_deref(),
            Some("vauchi://device-link?qr=AAAA&code=MDkx")
        );
    }

    // @internal
    #[test]
    fn a_screen_without_a_complete_device_link_invitation_yields_none() {
        assert_eq!(
            device_link_invitation_in("Generating link... Progress 25%"),
            None
        );
        assert_eq!(
            device_link_invitation_in("│  vauchi://device-link?qr=AAAA   │"),
            None
        );
    }

    // @internal
    #[test]
    fn the_confirmation_code_is_the_three_dash_three_digit_token() {
        assert_eq!(
            confirmation_code_in("│Device Wants to Link│\n│  042-917  │\n│Expires at 1792731197│")
                .as_deref(),
            Some("042-917")
        );
        assert_eq!(confirmation_code_in("Progress 25% 2026-10-23"), None);
    }
}
