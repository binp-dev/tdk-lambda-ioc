mod conn;
mod mux;

use request_channel::Requester;
use std::{io, string::FromUtf8Error, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedReceiver as Receiver;

pub type Addr = u8;
pub type Cmd = String;
pub type CmdRes = String;

pub const LINE_TERM: u8 = b'\r';

#[derive(Error, Debug)]
pub enum Error {
    #[error("I/O Error: {0}")]
    Io(#[from] io::Error),
    #[error("Parse: {0}")]
    Decode(#[from] FromUtf8Error),
    #[error("Timeout")]
    Timeout,
    #[error("Unexpected response from device: {0}")]
    Device(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Queued,
    Immediate,
}

#[derive(Debug, Clone)]
pub enum Signal {
    On,
    Off,
    Intr,
}

#[derive(Debug, Clone)]
struct ImmTx {
    addr: Addr,
    cmd: Cmd,
}
#[derive(Debug, Clone)]
enum QueTx {
    Cmd(Cmd),
    Yield,
}
type Rx = CmdRes;

pub struct Handle {
    pub req: Commander,
    pub sig: Receiver<Signal>,
}

const CMD_DELAY: Duration = Duration::from_millis(10);
const CMD_TIMEOUT: Duration = Duration::from_millis(200);
const CMD_RETRIES: usize = 2;

pub struct Commander {
    addr: Addr,
    imm: Arc<Requester<ImmTx, Rx>>,
    que: Requester<QueTx, Rx>,
}

impl Commander {
    pub async fn execute(&self, cmd: Cmd, priority: Priority) -> Option<CmdRes> {
        match priority {
            Priority::Immediate => self
                .imm
                .request(ImmTx {
                    addr: self.addr,
                    cmd,
                })
                .unwrap(),
            Priority::Queued => self.que.request(QueTx::Cmd(cmd)).unwrap(),
        }
        .get_response()
        .await
    }
    pub fn yield_(&self) {
        // Don't wait for response.
        let _ = self.que.request(QueTx::Yield).unwrap();
    }
}

use conn::AddrConnection;
pub use mux::Multiplexer;
