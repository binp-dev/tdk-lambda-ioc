mod conn;
use conn::*;

use request_channel::{channel as request_channel, Requester, Responder};
use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    string::FromUtf8Error,
    sync::Arc,
};
use thiserror::Error;
use tokio::{
    io::{split, AsyncRead, AsyncWrite},
    runtime, select,
    sync::{mpsc::unbounded_channel as channel, Notify},
};

pub type Addr = u8;
pub type Cmd = String;
pub type CmdRes = String;

pub const LINE_TERM: u8 = b'\r';

#[derive(Error, Debug)]
pub enum Error {
    #[error("I/O Error: {0}")]
    Io(#[from] io::Error),
    #[error("Parse: {0}")]
    Parse(#[from] FromUtf8Error),
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
    pub intr: Interrupt,
}

pub struct Commander {
    addr: Addr,
    imm: Arc<Requester<ImmTx, Rx>>,
    que: Requester<QueTx, Rx>,
}

impl Commander {
    pub async fn execute(&self, cmd: Cmd, priority: Priority) -> CmdRes {
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
        .unwrap()
    }
    pub fn yield_(&self) {
        // Don't wait for response.
        let _ = self.que.request(QueTx::Yield).unwrap();
    }
}

pub type Interrupt = Arc<Notify>;

struct Client {
    resp: Responder<QueTx, Rx>,
    intr: Interrupt,
}

pub struct Multiplexer<Port: AsyncRead + AsyncWrite + Unpin> {
    port: Port,
    clients: HashMap<Addr, Client>,
    imm: Responder<ImmTx, Rx>,
    imm_req: Arc<Requester<ImmTx, Rx>>,
}

impl<Port: AsyncRead + AsyncWrite + Unpin> Multiplexer<Port> {
    pub fn new(port: Port) -> Self {
        let (req, resp) = request_channel::<ImmTx, Rx>();
        Self {
            port,
            imm: resp,
            imm_req: Arc::new(req),
            clients: HashMap::new(),
        }
    }

    pub fn add_client(&mut self, addr: Addr) -> Option<Handle> {
        let vacant = match self.clients.entry(addr) {
            Entry::Vacant(vacant) => vacant,
            Entry::Occupied(..) => return None,
        };
        let (req, resp) = request_channel::<QueTx, Rx>();
        let intr = Arc::new(Notify::new());
        vacant.insert(Client {
            resp,
            intr: intr.clone(),
        });
        Some(Handle {
            req: Commander {
                addr,
                imm: self.imm_req.clone(),
                que: req,
            },
            intr,
        })
    }

    pub async fn run(mut self) -> ! {
        let (mut clients, client_intrs): (HashMap<_, _>, HashMap<_, _>) = {
            self.clients
                .into_iter()
                .map(|(addr, client)| ((addr, client.resp), (addr, client.intr)))
                .unzip()
        };

        let (intr_sender, mut intr) = channel::<Addr>();
        runtime::Handle::current().spawn(async move {
            let clients = client_intrs;
            loop {
                let addr = intr.recv().await.unwrap();
                log::trace!("Intr: {}", addr);
                clients[&addr].notify_one();
            }
        });

        let mut conn = Connection::new(split(self.port), intr_sender);

        let mut sched = {
            let mut addrs = clients.keys().copied().collect::<Vec<_>>();
            addrs.sort();
            addrs.into_iter().cycle()
        };
        let mut current = sched.next().unwrap();
        let mut active = None;
        loop {
            let (addr, cmd, r) = select! {
                // Read immediate commands from all clients
                imm = self.imm.next() => {
                    let (ImmTx { addr, cmd }, r) = imm.unwrap();
                    (addr, cmd, r)
                },
                // Read queued commans from current client
                que = clients.get_mut(&current.clone()).unwrap().next() => {
                    let (tx, r) = que.unwrap();
                    match tx {
                        QueTx::Cmd(cmd) => (current, cmd, r),
                        QueTx::Yield => {
                            current = sched.next().unwrap();
                            continue;
                        },
                    }
                },
            };

            // Switch active address if needed
            if !active.map(|a| addr == a).unwrap_or(false) {
                match conn
                    .request(&format!("ADR {}", addr))
                    .await
                    .and_then(|resp| {
                        if resp == "OK" {
                            Ok(())
                        } else {
                            Err(Error::Device(resp))
                        }
                    }) {
                    Ok(()) => {
                        active.replace(addr);
                    }
                    Err(err) => {
                        log::error!("Cannot set device address {}: {:?}", addr, err);
                        continue;
                    }
                }
            }

            // Execute command
            match conn.request(&cmd).await {
                Ok(resp) => {
                    r.respond(resp);
                }
                Err(err) => {
                    log::error!("Cannot run command '{}': {:?}", cmd, err);
                }
            }
        }
    }
}
