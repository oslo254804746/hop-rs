use std::{
    io::{Result as IoResult, Write},
    sync::{Arc, Mutex},
};

use ratatui::{backend::CrosstermBackend, Terminal};

pub type SshTerminal = Terminal<CrosstermBackend<TerminalHandle>>;

#[derive(Clone)]
pub struct TerminalHandle {
    inner: Arc<Mutex<TerminalOutput>>,
}

#[derive(Default)]
struct TerminalOutput {
    sink: Vec<u8>,
    output: Vec<u8>,
}

impl TerminalHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TerminalOutput::default())),
        }
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        let mut inner = self.inner.lock().expect("terminal output lock poisoned");
        let mut sink = std::mem::take(&mut inner.sink);
        inner.output.append(&mut sink);
        std::mem::take(&mut inner.output)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self::new()
    }
}

impl Write for TerminalHandle {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.inner
            .lock()
            .expect("terminal output lock poisoned")
            .sink
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        let mut inner = self.inner.lock().expect("terminal output lock poisoned");
        let mut sink = std::mem::take(&mut inner.sink);
        inner.output.append(&mut sink);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_buffers_output_for_ordered_session_writes() {
        let mut handle = TerminalHandle::new_for_test();

        handle.write_all(b"\x1b[?1049l").unwrap();
        handle.flush().unwrap();

        assert_eq!(handle.take_output(), b"\x1b[?1049l");
    }
}
