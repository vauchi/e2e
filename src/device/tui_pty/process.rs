// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::Duration;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub(super) struct TerminalProcess {
    output: Receiver<std::io::Result<Vec<u8>>>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    _master: Box<dyn MasterPty + Send>,
}

impl TerminalProcess {
    pub(super) fn new(
        binary: &std::path::Path,
        data_dir: &std::path::Path,
        relay_url: &str,
        extra_env: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<Self> {
        let pair = native_pty_system().openpty(PtySize {
            rows: 40,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let mut command = CommandBuilder::new(binary);
        command.env("TERM", "xterm-256color");
        command.env("VAUCHI_DATA_DIR", data_dir);
        command.env("VAUCHI_RELAY_URL", relay_url);
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        // Bound queued output while the caller is inspecting a retained screen.
        let (sender, output) = sync_channel(16);
        let process = Self {
            output,
            child,
            writer,
            _master: pair.master,
        };
        std::thread::Builder::new()
            .name("e2e-terminal-output".into())
            .spawn(move || {
                let mut buffer = [0; 4096];
                loop {
                    let chunk = reader
                        .read(&mut buffer)
                        .map(|count| buffer[..count].to_vec());
                    let finished = !matches!(&chunk, Ok(bytes) if !bytes.is_empty());
                    if sender.send(chunk).is_err() || finished {
                        break;
                    }
                }
            })?;
        Ok(process)
    }

    pub(super) fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub(super) fn read(&self, timeout: Duration) -> std::io::Result<Option<Vec<u8>>> {
        match self.output.recv_timeout(timeout) {
            Ok(chunk) => chunk.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Ok(Some(Vec::new())),
        }
    }
}

impl Drop for TerminalProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
