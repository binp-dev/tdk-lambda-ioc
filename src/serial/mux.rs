use super::*;
use request_channel::{channel as request_channel, Requester, Responder, Response};
use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    ops::Deref,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{split, AsyncRead, AsyncWrite},
    runtime, select,
    sync::mpsc::{unbounded_channel as channel, UnboundedSender as Sender},
    time::{sleep, sleep_until, Instant},
};

const YIELD_TIMEOUT: Duration = Duration::from_millis(1000);

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

    pub fn add_client(&mut self, addr: Addr) -> Option<SerialHandle> {
        let vacant = match self.clients.entry(addr) {
            Entry::Vacant(vacant) => vacant,
            Entry::Occupied(..) => return None,
        };
        let (req, resp) = request_channel::<QueTx, Rx>();
        let (sig_send, sig_recv) = channel::<Signal>();
        vacant.insert(Client {
            resp,
            sig: sig_send,
        });
        Some(SerialHandle {
            req: Arc::new(Commander {
                addr,
                imm: self.imm_req.clone(),
                que: req,
            }),
            sig: sig_recv,
        })
    }
}

impl<Port: AsyncRead + AsyncWrite + Unpin> Multiplexer<Port> {
    pub async fn run(mut self) -> ! {
        let (mut clients, client_intrs): (HashMap<_, _>, HashMap<_, _>) = {
            self.clients
                .into_iter()
                .map(|(addr, client)| {
                    let sig = client.sig.clone();
                    ((addr, client), (addr, sig))
                })
                .unzip()
        };

        let (intr_send, intr_recv) = channel::<Addr>();
        runtime::Handle::current().spawn(async move {
            let clients = client_intrs;
            let mut intr = intr_recv;
            loop {
                let addr = intr.recv().await.unwrap();
                log::trace!("Intr: {}", addr);
                match clients.get(&addr) {
                    Some(client) => client.send(Signal::Intr).unwrap(),
                    None => log::error!("No client for interrupt: {}", addr),
                }
            }
        });

        let mut conn = AddrConnection::new(split(self.port), intr_send);

        // Main loop
        let mut sched = Scheduler::new(clients.keys().copied());
        loop {
            select! {
                biased;
                // Read immediate commands from all clients
                req = self.imm.next() => request_immediate(&mut conn, req).await,
                // Read queued commands from current client
                (cur, req, sig) = get_queued(&mut sched, &mut clients) => {
                    if cur.is_online() {
                        request_queued(&mut conn, cur, req, sig).await;
                    } else {
                        check_online(&mut conn, cur, sig).await;
                    }
                },
            }
        }
    }
}

async fn request_immediate(
    conn: &mut AddrConnection<impl AsyncWrite + Unpin, impl AsyncRead + Unpin>,
    req: Option<(ImmTx, Response<'_, String>)>,
) {
    let (ImmTx { addr, cmd }, r) = req.unwrap();
    match conn.request(addr, &cmd).await {
        Ok(resp) => {
            r.respond(resp);
        }
        Err(err) => {
            log::error!(
                "Device {} failed to execute immediate command '{}': {}",
                addr,
                &cmd,
                err
            );
        }
    }
}

async fn get_queued<'a, 'b>(
    sched: &'a mut Scheduler,
    clients: &'b mut HashMap<Addr, Client>,
) -> (
    SchedGuard<'a>,
    Option<(QueTx, Response<'b, String>)>,
    &'b Sender<Signal>,
) {
    let cur = sched.current().await.unwrap();
    let addr = *cur;
    let client = clients.get_mut(&addr).unwrap();
    let req = if cur.is_online() {
        select! {
            biased;
            req = client.resp.next() => req,
            () = sleep(YIELD_TIMEOUT) => {
                log::warn!("Yield timeout reached");
                None
            }
        }
    } else {
        // Clear all pending requests
        while client.resp.try_next().map(|r| r.unwrap()).is_some() {}
        None
    };
    (cur, req, &client.sig)
}

async fn request_queued(
    conn: &mut AddrConnection<impl AsyncWrite + Unpin, impl AsyncRead + Unpin>,
    cur: SchedGuard<'_>,
    req: Option<(QueTx, Response<'_, String>)>,
    sig: &Sender<Signal>,
) {
    let addr = *cur;
    match req {
        Some((QueTx::Cmd(cmd), r)) => match conn.request(addr, &cmd).await {
            Ok(resp) => {
                r.respond(resp);
            }
            Err(err) => {
                log::error!(
                    "Device {} failed to execute queued command '{}', switching off: {}",
                    addr,
                    &cmd,
                    err
                );
                sig.send(Signal::Off).unwrap();
                cur.yield_offline();
            }
        },
        None | Some((QueTx::Yield, ..)) => {
            cur.yield_online();
        }
    }
}

async fn check_online(
    conn: &mut AddrConnection<impl AsyncWrite + Unpin, impl AsyncRead + Unpin>,
    cur: SchedGuard<'_>,
    sig: &Sender<Signal>,
) {
    let addr = *cur;
    log::debug!("Check device {} online", addr);
    match conn.is_online(addr).await {
        Ok(true) => {
            sig.send(Signal::On).unwrap();
            cur.yield_online();
        }
        Ok(false) => cur.yield_offline(),
        Err(err) => {
            log::error!("Error while checking device {}: {}", addr, err);
            cur.yield_offline();
        }
    }
}

struct Scheduler {
    current: Option<(Addr, bool)>,
    online: VecDeque<Addr>,
    offline: VecDeque<(Instant, Addr)>,
    counter: usize,
}

impl Scheduler {
    pub fn new<I: IntoIterator<Item = Addr>>(addrs: I) -> Self {
        let now = Instant::now();
        let mut offline: VecDeque<_> = addrs.into_iter().map(|a| (now, a)).collect();
        offline.as_mut_slices().0.sort();
        Self {
            current: None,
            online: VecDeque::new(),
            offline,
            counter: 0,
        }
    }

    async fn current(&mut self) -> Option<SchedGuard<'_>> {
        if self.current.is_none() {
            self.current.replace(match self.offline.pop_front() {
                Some((ts, a)) => {
                    if ts <= Instant::now() && (self.counter % 2 == 0 || self.online.is_empty()) {
                        (a, false)
                    } else {
                        match self.online.pop_front() {
                            Some(oa) => {
                                self.offline.push_front((ts, a));
                                (oa, true)
                            }
                            None => {
                                sleep_until(ts).await;
                                (a, false)
                            }
                        }
                    }
                }
                None => (self.online.pop_front()?, true),
            });
            self.counter += 1;
        }
        Some(SchedGuard { owner: self })
    }
}

struct SchedGuard<'a> {
    owner: &'a mut Scheduler,
}

impl<'a> SchedGuard<'a> {
    fn current(&self) -> &(Addr, bool) {
        self.owner.current.as_ref().unwrap()
    }
    fn take_current(&mut self) -> (Addr, bool) {
        self.owner.current.take().unwrap()
    }

    pub fn is_online(&self) -> bool {
        self.current().1
    }

    pub fn yield_online(mut self) {
        let addr = self.take_current().0;
        self.owner.online.push_back(addr);
    }
    pub fn yield_offline(mut self) {
        let addr = self.take_current().0;
        let ts = Instant::now() + Duration::from_secs(1);
        self.owner.offline.push_back((ts, addr));
    }
}

impl<'a> Deref for SchedGuard<'a> {
    type Target = Addr;
    fn deref(&self) -> &Addr {
        &self.current().0
    }
}
