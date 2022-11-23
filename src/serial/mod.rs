mod conn;
use conn::*;

use futures::Stream;
use request_channel::{channel as request_channel, Requester, Responder};
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap, VecDeque},
    io,
    string::FromUtf8Error,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::{
    io::{split, AsyncRead, AsyncWrite},
    runtime, select,
    sync::{
        mpsc::{
            unbounded_channel as channel, UnboundedReceiver as Receiver, UnboundedSender as Sender,
        },
        Notify,
    },
    time::{sleep_until, Instant},
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
    pub sig: Receiver<Signal>,
}

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

pub type Interrupt = Arc<Notify>;

struct Client {
    resp: Responder<QueTx, Rx>,
    intr: Interrupt,
    sig: Sender<Signal>,
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
        let (sig_send, sig_recv) = channel();
        vacant.insert(Client {
            resp,
            intr: intr.clone(),
            sig: sig_send,
        });
        Some(Handle {
            req: Commander {
                addr,
                imm: self.imm_req.clone(),
                que: req,
            },
            intr,
            sig: sig_recv,
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

        let mut sched = Scheduler::new(clients.keys().copied());
        let mut next = None;
        let mut active = None;
        loop {
            let client_fut = async {
                let cur = match next {
                    Some(a) => a,
                    None => {
                        let addr = sched.next().await.unwrap();
                        next.replace(addr);
                        addr
                    }
                };
                (cur, clients.get_mut(&cur).unwrap().next().await)
            };

            let (addr, cmd, r) = select! {
                // Read immediate commands from all clients
                imm = self.imm.next() => {
                    let (ImmTx { addr, cmd }, r) = imm.unwrap();
                    (addr, cmd, r)
                },
                // Read queued commands from current client
                (cur, que) = client_fut => {
                    match que {
                        Some((QueTx::Cmd(cmd), r)) => (cur, cmd, r),
                        None | Some((QueTx::Yield, ..)) => {
                            sched.schedule_active(cur);
                            next = None;
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
                        log::error!("Cannot set device address {}: {}", addr, err);
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
                    log::error!("Cannot run command '{}': {}", cmd, err);
                }
            }
        }
    }
}

struct Scheduler {
    active: VecDeque<Addr>,
    offline: VecDeque<(Instant, Addr)>,
}

impl Scheduler {
    pub fn new<I: IntoIterator<Item = Addr>>(addrs: I) -> Self {
        let mut active: VecDeque<_> = addrs.into_iter().collect();
        active.as_mut_slices().0.sort();
        Self {
            active,
            offline: VecDeque::new(),
        }
    }

    async fn next(&mut self) -> Option<Addr> {
        if let Some(a) = self.active.pop_front() {
            return Some(a);
        }
        let (ts, a) = self.offline.pop_front()?;
        sleep_until(ts).await;
        Some(a)
    }

    fn schedule_active(&mut self, addr: Addr) {
        self.active.push_back(addr);
    }
    fn schedule_offline(&mut self, addr: Addr) {
        self.offline
            .push_back((Instant::now() + Duration::from_secs(1), addr));
    }
}
