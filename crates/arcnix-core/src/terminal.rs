//! Interactive PTY on an exclusively owned SSH transport. No command execution
//! shortcuts or local shell: input/output are the remote terminal's raw bytes.
use std::collections::VecDeque;
use std::io::{self, Read, Write};

use anyhow::{Context, Result, ensure};
use ssh2::{Channel, ErrorCode, ExtendedData, Session};

const INPUT_LIMIT: usize = 1024 * 1024;
pub struct Pty {
    session: Session,
    channel: Channel,
    input: VecDeque<Vec<u8>>,
    input_offset: usize,
    pending_bytes: usize,
    resize: Option<(u32, u32)>,
}
pub struct Output {
    pub bytes: Vec<u8>,
    pub exit_code: Option<i32>,
}
fn dimensions(columns: u32, rows: u32) -> Result<()> {
    ensure!(
        (2..=500).contains(&columns) && (1..=300).contains(&rows),
        "Terminal dimensions are out of range"
    );
    Ok(())
}
fn again(error: &ssh2::Error) -> bool {
    error.code() == ErrorCode::Session(-37)
}
impl Pty {
    pub(crate) fn open(session: Session, columns: u32, rows: u32) -> Result<Self> {
        dimensions(columns, rows)?;
        let mut channel = session
            .channel_session()
            .context("Cannot open SSH terminal channel")?;
        channel.handle_extended_data(ExtendedData::Merge)?;
        channel
            .request_pty("xterm-256color", None, Some((columns, rows, 0, 0)))
            .context("Server refused a terminal PTY")?;
        channel
            .shell()
            .context("Server refused an interactive shell")?;
        session.set_blocking(false);
        Ok(Self {
            session,
            channel,
            input: VecDeque::new(),
            input_offset: 0,
            pending_bytes: 0,
            resize: None,
        })
    }
    pub fn input(&mut self, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            bytes.len() <= 16_384 && self.pending_bytes.saturating_add(bytes.len()) <= INPUT_LIMIT,
            "Terminal input queue is full"
        );
        if !bytes.is_empty() {
            self.pending_bytes += bytes.len();
            self.input.push_back(bytes);
        }
        Ok(())
    }
    pub fn resize(&mut self, columns: u32, rows: u32) -> Result<()> {
        dimensions(columns, rows)?;
        self.resize = Some((columns, rows));
        Ok(())
    }
    /// Bounded nonblocking work. A zero read budget still flushes input and
    /// resize messages, so output backpressure never prevents Ctrl+C.
    pub fn poll(&mut self, read_budget: usize) -> Result<Output> {
        if let Some((columns, rows)) = self.resize {
            match self.channel.request_pty_size(columns, rows, None, None) {
                Ok(()) => self.resize = None,
                Err(error) if again(&error) => {}
                Err(error) => return Err(error).context("Cannot resize terminal"),
            }
        }
        let mut written = 0;
        while let Some(bytes) = self.input.front() {
            if written >= 65_536 {
                break;
            }
            match self.channel.write(&bytes[self.input_offset..]) {
                Ok(0) => break,
                Ok(count) => {
                    self.input_offset += count;
                    self.pending_bytes -= count;
                    written += count;
                    if self.input_offset == bytes.len() {
                        self.input.pop_front();
                        self.input_offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("Cannot write terminal input"),
            }
        }
        let mut output = Vec::new();
        let mut buffer = [0u8; 8192];
        while output.len() < read_budget.min(65_536) {
            let remaining = buffer.len().min(read_budget.min(65_536) - output.len());
            match self.channel.read(&mut buffer[..remaining]) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("Cannot read terminal output"),
            }
        }
        let exit_code = if self.channel.eof() {
            match self.channel.wait_close() {
                Ok(()) => Some(self.channel.exit_status()?),
                Err(error) if again(&error) => None,
                Err(error) => return Err(error).context("Cannot close terminal channel"),
            }
        } else {
            None
        };
        Ok(Output {
            bytes: output,
            exit_code,
        })
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.channel.close();
        let _ = self.session.disconnect(None, "Terminal closed", None);
    }
}
