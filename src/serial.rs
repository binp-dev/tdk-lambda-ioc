use pin_project::pin_project;
use request_channel::{channel as request_channel, Requester, Responder};
use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{split, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf},
    runtime, select,
    sync::{
        mpsc::{unbounded_channel as channel, UnboundedSender as Sender},
        Notify,
    },
    time::sleep,
};

pub type Addr = u8;
pub type Cmd = String;
pub type CmdRes = String;

pub const LINE_TERM: u8 = b'\r';

fn byte_is_intr(b: u8) -> Option<Addr> {
    if b >= 0x80 {
        Some(b - 0x80)
    } else {
        None
    }
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

pub struct Multiplexer<Port: AsyncRead + AsyncWrite> {
    port: Port,
    clients: HashMap<Addr, Client>,
    imm: Responder<ImmTx, Rx>,
    imm_req: Arc<Requester<ImmTx, Rx>>,
}

impl<Port: AsyncRead + AsyncWrite> Multiplexer<Port> {
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

        let (intr_sender, mut intr_receiver) = channel::<Addr>();
        let (mut reader, mut writer) = {
            let (r, w) = split(self.port);
            let br = BufReader::new(FilterReader::new(r, intr_sender));
            (br, w)
        };
        runtime::Handle::current().spawn(async move {
            let clients = client_intrs;
            loop {
                let addr = intr_receiver.recv().await.unwrap();
                log::trace!("Intr: {}", addr);
                clients[&addr].notify_one();
            }
        });

        let mut sched = {
            let mut addrs = clients.keys().copied().collect::<Vec<_>>();
            addrs.sort();
            addrs.into_iter().cycle()
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

            if !active.map(|a| addr == a).unwrap_or(false) {
                sleep(Duration::from_millis(10)).await;

                let cmd = format!("ADR {}", addr);
                writer.write_all(cmd.as_bytes()).await.unwrap();
                writer.write_u8(LINE_TERM).await.unwrap();
                writer.flush().await.unwrap();
                log::trace!("-> '{}'", cmd);

                buf.clear();
                reader.read_until(LINE_TERM, &mut buf).await.unwrap();
                assert_eq!(buf.pop().unwrap(), LINE_TERM);
                log::trace!("<- '{}'", String::from_utf8_lossy(&buf));
                assert_eq!(buf, b"OK");
                active.replace(addr);
            }

            sleep(Duration::from_millis(10)).await;

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

#[pin_project]
struct FilterReader<R: AsyncRead> {
    #[pin]
    reader: R,
    prev: Option<Addr>,
    chan: Sender<Addr>,
}

impl<R: AsyncRead> FilterReader<R> {
    pub fn new(reader: R, intr_chan: Sender<Addr>) -> Self {
        Self {
            reader,
            prev: None,
            chan: intr_chan,
        }
    }
}

impl<R: AsyncRead> AsyncRead for FilterReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut this = self.project();
        let len = buf.filled().len();
        match AsyncRead::poll_read(Pin::new(&mut this.reader), cx, buf) {
            Poll::Ready(Ok(())) => {
                let s = &mut buf.filled_mut()[len..];
                if s.is_empty() {
                    // Stream is closed.
                    return Poll::Ready(Ok(()));
                }
                let mut j = 0;
                for i in 0..s.len() {
                    let b = s[i];
                    match this.prev.take() {
                        Some(p) => {
                            let a = byte_is_intr(b).unwrap();
                            assert_eq!(a, p);
                            this.chan.send(a).unwrap();
                        }
                        None => match byte_is_intr(b) {
                            Some(a) => {
                                this.prev.replace(a);
                            }
                            None => {
                                s[j] = b;
                                j += 1;
                            }
                        },
                    }
                }
                buf.set_filled(j);
                if j == 0 {
                    Poll::Pending
                } else {
                    Poll::Ready(Ok(()))
                }
            }
            poll => poll,
        }
    }
}
