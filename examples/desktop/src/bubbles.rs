//! Native text presentation owned by the desktop session, outside the animation loop.
use std::{
    io::{self, Read, Write},
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, Command, Stdio},
};

pub struct Bubbles {
    child: Child,
    input: Option<UnixStream>,
    pending: Vec<u8>,
    offset: usize,
    latest: Vec<u8>,
    previous: Vec<u8>,
    command: Vec<u8>,
    output: Vec<u8>,
}
pub enum UiCommand {
    Remove(u32),
    Work { window: u32, active: bool },
}
impl Bubbles {
    pub fn start(roles: &[(u32, String)]) -> io::Result<Self> {
        let script = crate::install_paths::bubble_host();
        let (input, reader) = UnixStream::pair()?;
        input.set_nonblocking(true)?;
        let child = Command::new("/usr/bin/python3")
            .arg("-B")
            .arg(script)
            .arg(serde_json::to_string(roles)?)
            .env("GDK_BACKEND", "x11")
            // EOF closes the helper even if the parent is killed abruptly.
            .stdin(Stdio::from(OwnedFd::from(reader)))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(Self {
            child,
            input: Some(input),
            pending: Vec::new(),
            offset: 0,
            latest: Vec::new(),
            previous: Vec::new(),
            command: Vec::new(),
            output: Vec::new(),
        })
    }

    pub fn context_menu(&mut self, window: u32, x: i16, y: i16, time: u32) {
        self.command =
            format!("{{\"type\":\"context_menu\",\"window\":{window},\"x\":{x},\"y\":{y},\"time\":{time}}}\n")
                .into_bytes();
        self.flush();
    }

    pub fn stream(&self) -> Option<&UnixStream> {
        self.input.as_ref()
    }

    /// Commands return only from this session's UI helper, never from the pointer stream.
    pub fn commands(&mut self) -> Vec<UiCommand> {
        let mut requests = Vec::new();
        let Some(input) = &mut self.input else {
            return requests;
        };
        let mut chunk = [0u8; 1024];
        loop {
            match input.read(&mut chunk) {
                Ok(0) => {
                    self.input = None;
                    break;
                }
                Ok(n) => {
                    self.output.extend_from_slice(&chunk[..n]);
                    if self.output.len() > 65536 {
                        self.output.clear();
                        break;
                    }
                    while let Some(end) = self.output.iter().position(|byte| *byte == b'\n') {
                        let line: Vec<_> = self.output.drain(..=end).collect();
                        if let Ok(message) = serde_json::from_slice::<serde_json::Value>(&line) {
                            if message["type"] == "remove_role" {
                                if let Some(window) = message["window"]
                                    .as_u64()
                                    .and_then(|id| u32::try_from(id).ok())
                                {
                                    requests.push(UiCommand::Remove(window));
                                }
                            } else if message["type"] == "work_state" {
                                if let (Some(window), Some(active)) = (
                                    message["window"]
                                        .as_u64()
                                        .and_then(|id| u32::try_from(id).ok()),
                                    message["active"].as_bool(),
                                ) {
                                    requests.push(UiCommand::Work { window, active });
                                }
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.input = None;
                    break;
                }
            }
        }
        requests
    }

    /// Coalesce pointer snapshots; a busy UI must never block pet animation.
    /// At most one partly written frame and one newest snapshot are retained.
    pub fn pointer(&mut self, anchor: Option<[i32; 4]>, point: Option<[f64; 2]>, blocked: bool) {
        let mut frame = serde_json::to_vec(&serde_json::json!({
            "type": "pointer", "anchor": anchor, "pointer": point, "blocked": blocked,
        }))
        .expect("finite desktop coordinates");
        frame.push(b'\n');
        if frame != self.previous {
            self.previous.clone_from(&frame);
            self.latest = frame;
        }
        self.flush();
    }

    fn flush(&mut self) {
        for _ in 0..2 {
            if self.pending.is_empty() {
                self.pending = if self.command.is_empty() {
                    std::mem::take(&mut self.latest)
                } else {
                    std::mem::take(&mut self.command)
                };
                self.offset = 0;
            }
            if self.pending.is_empty() {
                break;
            }
            let Some(input) = &mut self.input else {
                break;
            };
            match input.write(&self.pending[self.offset..]) {
                Ok(0) => {
                    self.input = None;
                    break;
                }
                Ok(n) => {
                    self.offset += n;
                    if self.offset == self.pending.len() {
                        self.pending.clear();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.input = None;
                    break;
                }
            }
        }
    }
}
impl Drop for Bubbles {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn stalled_ui_coalesces_snapshots_without_blocking_and_eof_closes_session() {
        let (mut input, mut reader) = UnixStream::pair().unwrap();
        input.set_nonblocking(true).unwrap();
        let mut filled = 0;
        loop {
            match input.write(&[b' '; 4096]) {
                Ok(n) => filled += n,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                result => panic!("unexpected write: {result:?}"),
            }
        }
        let mut bubbles = Bubbles {
            child: Command::new("/bin/true").spawn().unwrap(),
            input: Some(input),
            pending: Vec::new(),
            offset: 0,
            latest: Vec::new(),
            previous: Vec::new(),
            command: Vec::new(),
            output: Vec::new(),
        };
        for n in 0..10_000 {
            bubbles.pointer(Some([1, 2, 100, 200]), Some([f64::from(n), 0.0]), false);
        }
        assert!(bubbles.pending.len() < 512 && bubbles.latest.len() < 512);
        reader.read_exact(&mut vec![0; filled]).unwrap();
        bubbles.pointer(Some([1, 2, 100, 200]), Some([9999.0, 0.0]), false);
        drop(bubbles);
        let mut frames = String::new();
        reader.read_to_string(&mut frames).unwrap();
        let frames: Vec<serde_json::Value> = frames
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1]["pointer"][0], 9999.0);
    }
}
