mod conn;
use conn::*;

use request_channel::{channel as request_channel, Requester, Responder};
use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    io,
    ops::Deref,
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
            sig: sig_send,
        });
        Some(Handle {
            req: Commander {
                addr,
                imm: self.imm_req.clone(),
                que: req,
            },
            sig: sig_recv,
        })
    }
}

impl<Port: AsyncRead + AsyncWrite + Unpin> Multiplexer<Port> {
    pub async fn run(mut self) -> ! {
        let (mut clients, client_intrs): (HashMap<_, _>, HashMap<_, _>) = {
            self.clients
                .into_iter()
                .map(|(addr, client)| ((addr, client.resp), (addr, client.sig)))
                .unzip()
        };

        let (intr_sender, mut intr) = channel::<Addr>();
        runtime::Handle::current().spawn(async move {
            let clients = client_intrs;
            loop {
                let addr = intr.recv().await.unwrap();
                log::trace!("Intr: {}", addr);
                clients[&addr].send(Signal::Intr);
            }
        });

        let mut conn = AddressedConnection::new(split(self.port), intr_sender);

        let mut sched = Scheduler::new(clients.keys().copied());
        loop {
            let client_fut = async {
                let cur = sched.current().await.unwrap();
                let addr = *cur;
                (cur, clients.get_mut(&addr).unwrap().next().await)
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
                        Some((QueTx::Cmd(cmd), r)) => (*cur, cmd, r),
                        None | Some((QueTx::Yield, ..)) => {
                            cur.yield_online();
                            continue;
                        },
                    }
                },
            };

            match conn.request(addr, &cmd).await {
                Ok(resp) => {
                    r.respond(resp);
                }
                Err(err) => {
                    log::error!("Cannot run command '{}': {}", &cmd, err);
                }
            }
        }
    }
}

struct Scheduler {
    current: Option<Addr>,
    online: VecDeque<Addr>,
    offline: VecDeque<(Instant, Addr)>,
}

impl Scheduler {
    pub fn new<I: IntoIterator<Item = Addr>>(addrs: I) -> Self {
        let mut online: VecDeque<_> = addrs.into_iter().collect();
        online.as_mut_slices().0.sort();
        Self {
            current: None,
            online,
            offline: VecDeque::new(),
        }
    }

    async fn current(&mut self) -> Option<SchedGuard<'_>> {
        if self.current.is_none() {
            let addr = if let Some(a) = self.online.pop_front() {
                a
            } else {
                let (ts, a) = self.offline.pop_front()?;
                sleep_until(ts).await;
                a
            };
            self.current.replace(addr);
        }
        Some(SchedGuard { owner: self })
    }
}

struct SchedGuard<'a> {
    owner: &'a mut Scheduler,
}

impl<'a> SchedGuard<'a> {
    fn yield_online(self) {
        let addr = self.owner.current.take().unwrap();
        self.owner.online.push_back(addr);
    }

    fn yield_offline(self) {
        let addr = self.owner.current.take().unwrap();
        let ts = Instant::now() + Duration::from_secs(1);
        self.owner.offline.push_back((ts, addr));
    }
}

impl<'a> Deref for SchedGuard<'a> {
    type Target = Addr;
    fn deref(&self) -> &Addr {
        self.owner.current.as_ref().unwrap()
    }
}
