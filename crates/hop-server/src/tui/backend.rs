use std::io::{Result as IoResult, Write};

use ratatui::{backend::CrosstermBackend, Terminal};
use russh::{server::Handle, ChannelId};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

pub type SshTerminal = Terminal<CrosstermBackend<TerminalHandle>>;

pub struct TerminalHandle {
    sender: UnboundedSender<Vec<u8>>,
    sink: Vec<u8>,
}

impl TerminalHandle {
    pub fn start(handle: Handle, channel_id: ChannelId) -> Self {
        let (sender, mut receiver) = unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(data) = receiver.recv().await {
                let _ = handle.data(channel_id, data).await;
            }
        });
        Self {
            sender,
            sink: Vec::new(),
        }
    }
}

impl Write for TerminalHandle {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.sink.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        let data = std::mem::take(&mut self.sink);
        self.sender
            .send(data)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "ssh channel closed"))
    }
}
