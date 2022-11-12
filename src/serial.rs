use request_channel::{channel, Requester, Responder};
use std::{
    collections::{
        hash_map::{Entry, HashMap},
        VecDeque,
    },
    sync::Arc,
    time::Duration,
};
use tokio::{select, sync::Notify, time::sleep};
//use tokio_serial::SerialStream;

// Temporary placeholder.
type SerialStream = ();

pub type Addr = u8;
pub type Cmd = Vec<u8>;
pub type CmdRes = Vec<u8>;

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
    pub async fn run(&self, cmd: Cmd, priority: Priority) -> CmdRes {
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

pub struct Multiplexer {
    port: SerialStream,
    clients: HashMap<Addr, Client>,
    imm: Responder<ImmTx, Rx>,
    imm_req: Arc<Requester<ImmTx, Rx>>,
}

impl Multiplexer {
    pub fn new(port: SerialStream) -> Self {
        let (req, resp) = channel::<ImmTx, Rx>();
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
        let (req, resp) = channel::<QueTx, Rx>();
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
        let mut sched = {
            let mut addrs = self.clients.keys().copied().collect::<Vec<_>>();
            addrs.sort();
            addrs.into_iter().cycle()
        };
        let mut current = sched.next().unwrap();
        loop {
            let (addr, cmd, r) = select! {
                imm = self.imm.next() => {
                    let (ImmTx { addr, cmd }, r) = imm.unwrap();
                    (addr, cmd, r)
                },
                que = self.clients.get_mut(&current.clone()).unwrap().resp.next() => {
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
            log::info!("{}: {}", addr, String::from_utf8_lossy(&cmd));
            sleep(Duration::from_millis(100)).await;
            r.respond(Vec::new());
        }
    }
}
