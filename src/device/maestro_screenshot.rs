// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The payload a mobile shell shows as a QR, read back from a Maestro
//! screenshot (vauchi/private#300). The shell is not asked to expose the
//! payload any other way: a copy action would put a device-link key on the
//! OS clipboard, and an accessible element would recite it to every
//! accessibility service.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{ImageFormat, ImageReader, Limits};
use vauchi_core::qr_decode::{ScannerBackend, scan_qr_from_luma};

use crate::error::{E2eError, E2eResult};

/// Larger than any phone screen; refused before the pixels are decoded.
const MAX_SCREENSHOT_SIDE_PX: u32 = 4096;

/// The QR payload in a PNG screenshot. Raw rqrr, not the camera pipeline:
/// a screenshot is pixel-exact, and rqrr reports every code in the frame,
/// so a second QR on screen is caught rather than silently ignored.
pub(super) fn qr_payload_in(png: &[u8]) -> E2eResult<String> {
    let unreadable = |e: image::ImageError| E2eError::device(format!("unreadable screenshot: {e}"));
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SCREENSHOT_SIDE_PX);
    limits.max_image_height = Some(MAX_SCREENSHOT_SIDE_PX);
    let mut reader = ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
    reader.limits(limits);
    let screen = reader.decode().map_err(unreadable)?.to_luma8();

    let (width, height) = screen.dimensions();
    let mut payloads =
        scan_qr_from_luma(ScannerBackend::RqrrRaw, screen.as_raw(), width, height).decoded_all;
    payloads.dedup();
    match payloads.len() {
        0 => Err(E2eError::device("the screenshot shows no QR code")),
        1 => Ok(payloads.remove(0)),
        n => Err(E2eError::device(format!(
            "the screenshot shows {n} different QR codes"
        ))),
    }
}

/// The one `<name>.png` Maestro wrote somewhere under `output_dir`.
pub(super) fn screenshot_in(output_dir: &Path, name: &str) -> E2eResult<PathBuf> {
    let file_name = format!("{name}.png");
    let mut found = Vec::new();
    collect_named(output_dir, &file_name, &mut found)?;
    match found.len() {
        0 => Err(E2eError::device(format!("Maestro wrote no {file_name}"))),
        1 => Ok(found.remove(0)),
        n => Err(E2eError::device(format!(
            "Maestro wrote {n} {file_name} files"
        ))),
    }
}

fn collect_named(dir: &Path, file_name: &str, found: &mut Vec<PathBuf>) -> E2eResult<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| E2eError::device(format!("Failed to read Maestro output: {e}")))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| E2eError::device(format!("Failed to read Maestro output: {e}")))?;
        let kind = entry
            .file_type()
            .map_err(|e| E2eError::device(format!("Failed to read Maestro output: {e}")))?;
        if kind.is_dir() {
            collect_named(&entry.path(), file_name, found)?;
        } else if kind.is_file() && entry.file_name() == file_name {
            found.push(entry.path());
        }
    }
    Ok(())
}

// INLINE_TEST_REQUIRED: the readers are private to the device module
#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, ImageFormat, Luma};
    use std::io::Cursor;

    const MODULE_PX: u32 = 8;
    const QUIET_ZONE_MODULES: u32 = 4;

    fn qr_at(canvas: &mut GrayImage, payload: &str, left: u32, top: u32) {
        let code = qrcode::QrCode::new(payload.as_bytes()).expect("payload fits a QR");
        let width = code.width() as u32;
        for (index, color) in code.to_colors().into_iter().enumerate() {
            if color != qrcode::Color::Dark {
                continue;
            }
            let (x, y) = (index as u32 % width, index as u32 / width);
            for dy in 0..MODULE_PX {
                for dx in 0..MODULE_PX {
                    canvas.put_pixel(
                        left + (QUIET_ZONE_MODULES + x) * MODULE_PX + dx,
                        top + (QUIET_ZONE_MODULES + y) * MODULE_PX + dy,
                        Luma([0]),
                    );
                }
            }
        }
    }

    fn png(canvas: &GrayImage) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        canvas
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode PNG");
        bytes.into_inner()
    }

    fn screen() -> GrayImage {
        GrayImage::from_pixel(1080, 1920, Luma([255]))
    }

    const LINK: &str = "vauchi://device-link?v=1&d=AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

    // @internal
    #[test]
    fn a_qr_on_the_screen_reads_back_as_its_payload() {
        let mut canvas = screen();
        qr_at(&mut canvas, LINK, 180, 600);

        assert_eq!(qr_payload_in(&png(&canvas)).expect("QR decodes"), LINK);
    }

    // @internal
    #[test]
    fn a_screen_without_a_qr_is_an_error_not_an_empty_payload() {
        let error = qr_payload_in(&png(&screen())).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Device error: the screenshot shows no QR code"
        );
    }

    // @internal
    #[test]
    fn two_different_qrs_on_one_screen_are_refused_as_ambiguous() {
        let mut canvas = screen();
        qr_at(&mut canvas, LINK, 40, 100);
        qr_at(&mut canvas, "vauchi://exchange?other", 40, 1100);

        let error = qr_payload_in(&png(&canvas)).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Device error: the screenshot shows 2 different QR codes"
        );
    }

    // @internal
    #[test]
    fn bytes_that_are_not_a_png_are_refused() {
        for bytes in [&b""[..], b"not a png", &[0x89, b'P', b'N', b'G']] {
            let error = qr_payload_in(bytes).unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("Device error: unreadable screenshot:"),
                "{bytes:?} gave {error}"
            );
        }
    }

    // @internal
    #[test]
    fn a_screenshot_larger_than_any_phone_screen_is_refused_before_decoding() {
        let huge = GrayImage::from_pixel(MAX_SCREENSHOT_SIDE_PX + 1, 16, Luma([255]));

        let error = qr_payload_in(&png(&huge)).unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("Device error: unreadable screenshot:"),
            "got {error}"
        );
    }

    // @internal
    #[test]
    fn the_screenshot_is_found_wherever_maestro_nests_it() {
        let output = tempfile::tempdir().expect("output dir");
        let nested = output
            .path()
            .join("2026-10-24_070000")
            .join("takeScreenshot");
        std::fs::create_dir_all(&nested).expect("nest");
        std::fs::write(nested.join("device_link_qr.png"), b"png").expect("write");
        std::fs::write(nested.join("other.png"), b"png").expect("write");

        assert_eq!(
            screenshot_in(output.path(), "device_link_qr").expect("found"),
            nested.join("device_link_qr.png")
        );
    }

    // @internal
    #[test]
    fn a_flow_that_took_no_screenshot_names_the_one_it_expected() {
        let output = tempfile::tempdir().expect("output dir");

        let error = screenshot_in(output.path(), "device_link_qr").unwrap_err();

        assert_eq!(
            error.to_string(),
            "Device error: Maestro wrote no device_link_qr.png"
        );
    }

    // @internal
    #[test]
    fn two_screenshots_of_the_same_name_are_refused_as_ambiguous() {
        let output = tempfile::tempdir().expect("output dir");
        for run in ["a", "b"] {
            let dir = output.path().join(run);
            std::fs::create_dir_all(&dir).expect("nest");
            std::fs::write(dir.join("device_link_qr.png"), b"png").expect("write");
        }

        let error = screenshot_in(output.path(), "device_link_qr").unwrap_err();

        assert_eq!(
            error.to_string(),
            "Device error: Maestro wrote 2 device_link_qr.png files"
        );
    }
}
