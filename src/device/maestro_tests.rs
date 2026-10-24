// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn test_device(workspace: MaestroWorkspace) -> MaestroDevice {
    MaestroDevice {
        name: "test".to_string(),
        platform: MaestroPlatform::Ios,
        device_name: "test-device".to_string(),
        app_id: "app.vauchi.ios".to_string(),
        relay_url: "ws://localhost:8080".to_string(),
        flows_dir: PathBuf::new(),
        network_config: NetworkConfig::default(),
        workspace,
        program: PathBuf::from("maestro"),
    }
}

/// A stand-in for the Maestro CLI that, like a flow's `takeScreenshot`
/// and `console.log`, saves `screenshot` and writes `console` lines to
/// `maestro.log` under the run's `--test-output-dir`, nested the way
/// Maestro 2.8.0 nests its artifacts. It records the arguments it ran with.
struct FakeMaestro {
    dir: tempfile::TempDir,
}

impl FakeMaestro {
    fn saving(screenshot: Option<(&str, &str)>) -> Self {
        Self::running(screenshot, &[])
    }

    fn logging(console: &[&str]) -> Self {
        Self::running(None, console)
    }

    fn running(screenshot: Option<(&str, &str)>, console: &[&str]) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("fake Maestro dir");
        let save = match screenshot {
            Some((name, payload)) => {
                let png = dir.path().join("screen.png");
                std::fs::write(&png, qr_screen_png(payload)).expect("write screen");
                format!(
                    "mkdir -p \"$out/2026-10-24_092300/flow/takeScreenshot\"\n\
                     cp '{}' \"$out/2026-10-24_092300/flow/takeScreenshot/{name}.png\"\n",
                    png.display()
                )
            }
            None => String::new(),
        };
        let log = dir.path().join("maestro.log");
        let console: Vec<String> = console
            .iter()
            .map(|line| {
                format!("09:50:16.690 [ INFO] maestro.orchestra.Orchestra: JsConsole: {line}")
            })
            .collect();
        std::fs::write(&log, console.join("\n")).expect("write console");
        let args = dir.path().join("args");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nout=\nwhile [ $# -gt 0 ]; do\n  \
             if [ \"$1\" = --test-output-dir ]; then out=$2; shift; fi\n  shift\n\
             done\n[ -n \"$out\" ] || exit 64\n{save}\
             mkdir -p \"$out/2026-10-24_092300/flow/logs\"\n\
             cp '{}' \"$out/2026-10-24_092300/flow/logs/maestro.log\"\n",
            args.display(),
            log.display()
        );
        let program = dir.path().join("maestro");
        std::fs::write(&program, script).expect("write fake Maestro");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("make fake Maestro executable");
        Self { dir }
    }

    fn program(&self) -> PathBuf {
        self.dir.path().join("maestro")
    }

    /// The flow it last ran and the `-e` parameters it was given.
    fn last_run(&self) -> (String, Vec<String>) {
        let args = std::fs::read_to_string(self.dir.path().join("args")).expect("ran");
        let args: Vec<&str> = args.lines().collect();
        let params = args
            .windows(2)
            .filter(|pair| pair[0] == "-e")
            .map(|pair| pair[1].to_string())
            .filter(|param| !param.starts_with("VAUCHI_RELAY_URL="))
            .collect();
        let flow = Path::new(args.last().expect("flow path"))
            .file_stem()
            .expect("flow name")
            .to_string_lossy()
            .into_owned();
        (flow, params)
    }
}

fn qr_screen_png(payload: &str) -> Vec<u8> {
    use image::{GrayImage, ImageFormat, Luma};
    const MODULE_PX: u32 = 8;
    let code = qrcode::QrCode::new(payload.as_bytes()).expect("payload fits a QR");
    let width = code.width() as u32;
    let mut screen = GrayImage::from_pixel(1080, 1920, Luma([255]));
    for (index, color) in code.to_colors().into_iter().enumerate() {
        if color != qrcode::Color::Dark {
            continue;
        }
        let (x, y) = (index as u32 % width, index as u32 / width);
        for dy in 0..MODULE_PX {
            for dx in 0..MODULE_PX {
                screen.put_pixel(
                    200 + x * MODULE_PX + dx,
                    600 + y * MODULE_PX + dy,
                    Luma([0]),
                );
            }
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    screen
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("encode PNG");
    bytes.into_inner()
}

fn android_device(fake: &FakeMaestro, flows: &[&str]) -> (MaestroDevice, tempfile::TempDir) {
    let flows_dir = tempfile::tempdir().expect("flows dir");
    for flow in flows {
        std::fs::write(flows_dir.path().join(format!("{flow}.yaml")), "").expect("write flow");
    }
    let mut device = test_device(MaestroWorkspace::new().expect("workspace"));
    device.platform = MaestroPlatform::Android;
    device.flows_dir = flows_dir.path().to_path_buf();
    device.program = fake.program();
    (device, flows_dir)
}

const EXCHANGE: &str = "vauchi://exchange?d=AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
const INVITATION: &str = "vauchi://device-link?v=1&d=device-link-invitation-fixture";

// @internal
#[test]
fn test_maestro_every_flow_drives_the_app_the_harness_launches() {
    for platform in [MaestroPlatform::Ios, MaestroPlatform::Android] {
        let flows_dir = MaestroDevice::find_flows_dir(platform).expect("flows dir");
        let mut flows: Vec<_> = std::fs::read_dir(&flows_dir)
            .expect("read flows")
            .map(|entry| entry.expect("flow entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
            .collect();
        flows.sort();
        assert!(!flows.is_empty(), "no {platform} flows");
        for flow in flows {
            let yaml = std::fs::read_to_string(&flow).expect("read flow");
            let app_id = yaml
                .lines()
                .find_map(|line| line.strip_prefix("appId:"))
                .map(str::trim);
            assert_eq!(
                app_id,
                Some(platform.app_id()),
                "{} names another app",
                flow.display()
            );
        }
    }
}

// @internal
#[test]
fn test_maestro_platform_display() {
    assert_eq!(format!("{}", MaestroPlatform::Ios), "iOS");
    assert_eq!(format!("{}", MaestroPlatform::Android), "Android");
}

// @internal
#[test]
fn test_maestro_install_help_requires_reviewed_pinned_release() {
    assert_eq!(
        MAESTRO_INSTALL_HELP,
        "Install a reviewed, pinned release from https://github.com/mobile-dev-inc/Maestro/releases and verify it with maestro --version"
    );
    assert!(!MAESTRO_INSTALL_HELP.contains("| bash"));
    assert!(!MAESTRO_INSTALL_HELP.contains("| sh"));
}

// @internal
#[test]
fn test_maestro_platform_flag_ios_returns_ios() {
    // Check if Maestro is actually installed (needed to construct device)
    let maestro_installed = std::process::Command::new("maestro")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !maestro_installed {
        // Skip if Maestro not installed — tested via find_flows_dir path below
        return;
    }

    let device = MaestroDevice::ios("test", "iPhone 15 Pro", "ws://localhost:8080").unwrap();
    assert_eq!(device.platform_flag(), "ios");
}

// @internal
#[test]
fn test_maestro_platform_flag_android_returns_android() {
    let maestro_installed = std::process::Command::new("maestro")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !maestro_installed {
        return;
    }

    let device = MaestroDevice::android("test", "Pixel_7", "ws://localhost:8080").unwrap();
    assert_eq!(device.platform_flag(), "android");
}

// @internal
#[test]
fn test_maestro_find_flows_dir_ios_contains_ios_subdir() {
    let flows_dir = MaestroDevice::find_flows_dir(MaestroPlatform::Ios).unwrap();
    assert!(
        flows_dir.ends_with("maestro/ios"),
        "Expected flows dir to end with maestro/ios, got: {}",
        flows_dir.display()
    );
}

// @internal
#[test]
fn test_maestro_find_flows_dir_android_contains_android_subdir() {
    let flows_dir = MaestroDevice::find_flows_dir(MaestroPlatform::Android).unwrap();
    assert!(
        flows_dir.ends_with("maestro/android"),
        "Expected flows dir to end with maestro/android, got: {}",
        flows_dir.display()
    );
}

// @internal
#[test]
fn test_maestro_device_creation_depends_on_cli() {
    // Device creation depends on whether Maestro CLI is installed
    let result = MaestroDevice::ios("test", "iPhone 15 Pro", "ws://localhost:8080");

    // Check if Maestro is actually installed
    let maestro_installed = std::process::Command::new("maestro")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if maestro_installed {
        // If Maestro is installed, device creation should succeed
        assert!(result.is_ok(), "Expected Ok when Maestro is installed");
    } else {
        // If Maestro is not installed, device creation should fail
        assert!(
            result.is_err(),
            "Expected Err when Maestro is not installed"
        );
    }
}

// @internal
#[test]
fn test_maestro_workspaces_isolate_same_output() {
    let alice = MaestroWorkspace::new().expect("create Alice workspace");
    let bob = MaestroWorkspace::new().expect("create Bob workspace");

    let alice_contacts = alice.output_path(MaestroOutput::Contacts);
    let bob_contacts = bob.output_path(MaestroOutput::Contacts);

    assert_ne!(alice_contacts, bob_contacts);
    assert!(alice_contacts.starts_with(alice.directory.path()));
    assert!(bob_contacts.starts_with(bob.directory.path()));
}

// @internal
#[test]
fn test_maestro_prepare_output_removes_poisoned_result() {
    let workspace = MaestroWorkspace::new().expect("create Maestro workspace");
    let output_path = workspace.output_path(MaestroOutput::Card);
    std::fs::write(&output_path, br#"{"name":"Mallory"}"#).expect("write poisoned result");

    let prepared_path = workspace
        .prepare_output(MaestroOutput::Card)
        .expect("prepare Maestro output");

    assert_eq!(prepared_path, output_path);
    assert!(!prepared_path.exists());
}

// @internal
#[tokio::test]
async fn test_maestro_generate_qr_reads_the_payload_off_the_flows_screenshot() {
    let fake = FakeMaestro::saving(Some((EXCHANGE_QR_SCREENSHOT, EXCHANGE)));
    let (device, _flows) = android_device(&fake, &["generate_qr"]);

    assert_eq!(device.generate_qr().await.expect("payload"), EXCHANGE);
}

// @internal
#[tokio::test]
async fn test_maestro_start_device_link_reads_the_invitation_off_the_flows_screenshot() {
    let fake = FakeMaestro::saving(Some((DEVICE_LINK_QR_SCREENSHOT, INVITATION)));
    let (device, _flows) = android_device(&fake, &["link_device"]);

    assert_eq!(
        device.start_device_link().await.expect("invitation"),
        INVITATION
    );
}

// @internal
#[tokio::test]
async fn test_maestro_a_screenshot_from_an_earlier_run_is_never_read_back() {
    let saves = FakeMaestro::saving(Some((EXCHANGE_QR_SCREENSHOT, EXCHANGE)));
    let (mut device, _flows) = android_device(&saves, &["generate_qr"]);
    device.generate_qr().await.expect("first run saves a QR");

    let saves_nothing = FakeMaestro::saving(None);
    device.program = saves_nothing.program();
    let error = device.generate_qr().await.unwrap_err();

    assert_eq!(
        error.to_string(),
        "Device error: Maestro wrote no exchange_qr.png"
    );
}

// @internal
#[tokio::test]
async fn test_maestro_ios_generate_qr_is_unsupported_because_the_simulator_shows_a_link() {
    let device = test_device(MaestroWorkspace::new().expect("workspace"));

    let error = device.generate_qr().await.unwrap_err();

    match error {
        E2eError::DeviceNotSupported(message) => assert_eq!(
            message,
            "Maestro device: the iOS simulator exchanges by link, not QR (Glance needs Bluetooth)"
        ),
        other => panic!("expected DeviceNotSupported, got {other:?}"),
    }
}

// @internal
#[tokio::test]
async fn test_maestro_join_identity_opens_the_invitation_and_returns_the_code_it_shows() {
    let fake = FakeMaestro::logging(&["VAUCHI_OUTPUT confirmation_code=224-580"]);
    let (device, _flows) = android_device(&fake, &["join_identity"]);

    let code = device
        .join_identity(INVITATION, "Rig Phone")
        .await
        .expect("joined");

    assert_eq!(code, "224-580");
    assert_eq!(
        fake.last_run(),
        (
            "join_identity".to_string(),
            vec![
                format!("INVITATION={INVITATION}"),
                "DEVICE_NAME=Rig Phone".to_string(),
            ]
        )
    );
}

// @internal
#[tokio::test]
async fn test_maestro_join_identity_fails_when_the_flow_shows_no_code() {
    let fake = FakeMaestro::logging(&[]);
    let (device, _flows) = android_device(&fake, &["join_identity"]);

    let error = device
        .join_identity(INVITATION, "Rig Phone")
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Device error: the flow logged no confirmation_code"
    );
}

// @internal
#[tokio::test]
async fn test_maestro_complete_device_link_approves_only_the_code_the_joiner_shows() {
    let fake = FakeMaestro::logging(&[]);
    let (device, _flows) = android_device(&fake, &["complete_device_link"]);

    let code = device
        .complete_device_link("224-580")
        .await
        .expect("approved");

    assert_eq!(code, "224-580");
    assert_eq!(
        fake.last_run(),
        (
            "complete_device_link".to_string(),
            vec!["EXPECTED_CODE=224-580".to_string()]
        )
    );
}

// @internal
#[tokio::test]
async fn test_maestro_finish_device_join_waits_on_the_joining_device() {
    let fake = FakeMaestro::logging(&[]);
    let (device, _flows) = android_device(&fake, &["finish_device_join"]);

    device.finish_device_join("224-580").await.expect("joined");

    assert_eq!(fake.last_run(), ("finish_device_join".to_string(), vec![]));
}

// @internal
#[tokio::test]
async fn test_maestro_ios_device_join_is_unsupported_until_its_flows_exist() {
    let device = test_device(MaestroWorkspace::new().expect("workspace"));
    let expected = "Maestro device: the iOS device-link joining flows are not written yet";

    for error in [
        device
            .join_identity(INVITATION, "Rig Phone")
            .await
            .unwrap_err(),
        device.complete_device_link("224-580").await.unwrap_err(),
        device.finish_device_join("224-580").await.unwrap_err(),
    ] {
        match error {
            E2eError::DeviceNotSupported(message) => assert_eq!(message, expected),
            other => panic!("expected DeviceNotSupported, got {other:?}"),
        }
    }
}
