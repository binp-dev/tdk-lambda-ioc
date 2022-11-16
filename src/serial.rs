use request_channel::{channel, Requester, Responder};
use std::{
    collections::hash_map::{Entry, HashMap},
    sync::Arc,
};
use tokio::{
    io::{split, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    select,
    sync::Notify,
};

pub type Addr = u8;
pub type Cmd = String;
pub type CmdRes = String;

pub const LINE_TERM: u8 = b'\r';

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

pub struct Multiplexer<Port: AsyncRead + AsyncWrite> {
    port: Port,
    clients: HashMap<Addr, Client>,
    imm: Responder<ImmTx, Rx>,
    imm_req: Arc<Requester<ImmTx, Rx>>,
}

impl<Port: AsyncRead + AsyncWrite> Multiplexer<Port> {
    pub fn new(port: Port) -> Self {
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
        let (mut reader, mut writer) = {
            let (r, w) = split(self.port);
            let br = BufReader::new(r);
            (br, w)
        };
        let mut buf = Vec::new();

        let mut current = sched.next().unwrap();
        let mut active = None;
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

            if !active.map(|a| addr == a).unwrap_or(false) {
                let cmd = format!("ADR {}", addr);
                writer.write_all(cmd.as_bytes()).await.unwrap();
                writer.write_u8(LINE_TERM).await.unwrap();
                writer.flush().await.unwrap();

                buf.clear();
                reader.read_until(LINE_TERM, &mut buf).await.unwrap();
                //reader.read_until(LINE_TERM, &mut buf).await.unwrap();
                assert_eq!(buf.pop().unwrap(), LINE_TERM);
                log::trace!("'{}' -> '{}'", cmd, String::from_utf8_lossy(&buf));
                assert_eq!(buf, b"OK");
                active.replace(addr);
            }

            writer.write_all(cmd.as_bytes()).await.unwrap();
            writer.write_u8(LINE_TERM).await.unwrap();
            writer.flush().await.unwrap();

            buf.clear();
            reader.read_until(LINE_TERM, &mut buf).await.unwrap();
            assert_eq!(buf.pop().unwrap(), LINE_TERM);
            log::trace!("'{}' -> '{}'", cmd, String::from_utf8_lossy(&buf));

            r.respond(String::from_utf8(buf.clone()).unwrap());
        }
    }
}
