use async_priority_channel::{
    unbounded as priority_channel, Receiver as PriorityReceiver, Sender as PrioritySender,
};
use std::{
    collections::{
        hash_map::{Entry, HashMap},
        VecDeque,
    },
    sync::atomic::{AtomicU32, Ordering},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{
    mpsc::{
        unbounded_channel as channel, UnboundedReceiver as Receiver, UnboundedSender as Sender,
    },
    Mutex, Notify,
};
use tokio_serial::SerialStream;

pub type Addr = u8;
pub type Msg = Vec<u8>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Queued,
    Immediate,
}

#[derive(Debug, Clone)]
struct Tx {
    id: u32,
    msg: Msg,
}

#[derive(Debug, Clone)]
struct Rx {
    id: u32,
    msg: Msg,
}

pub struct Handle {
    pub req: Requester,
    pub intr: Interrupt,
}

struct BufReceiver {
    channel: Receiver<Rx>,
    buffer: HashMap<u32, Msg>,
}

impl BufReceiver {
    pub fn new(channel: Receiver<Rx>) -> Self {
        Self {
            channel,
            buffer: HashMap::new(),
        }
    }
    async fn try_recv(&mut self, desired: u32) -> Option<Msg> {
        if let Some(msg) = self.buffer.remove(&desired) {
            return Some(msg);
        }
        let Rx { id, msg } = self.channel.recv().await.unwrap();
        if id == desired {
            return Some(msg);
        }
        self.buffer.insert(id, msg).unwrap();
        None
    }
    pub async fn recv(this: &Mutex<Self>, desired: u32) -> Msg {
        loop {
            if let Some(msg) = this.lock().await.try_recv(desired).await {
                break msg;
            }
        }
    }
}

pub struct Requester {
    addr: Addr,

    send: PrioritySender<(Tx, Addr), Priority>,
    cnt: AtomicU32,

    recv: Mutex<BufReceiver>,
}

impl Requester {
    fn new(addr: Addr, send: PrioritySender<(Tx, Addr), Priority>, recv: Receiver<Rx>) -> Self {
        Self {
            addr,
            send,
            cnt: AtomicU32::new(0),
            recv: Mutex::new(BufReceiver::new(recv)),
        }
    }
    pub async fn request(&self, msg: Msg, priority: Priority) -> Msg {
        let id = self.cnt.fetch_add(1, Ordering::SeqCst);
        self.send
            .send((Tx { id, msg }, self.addr), priority)
            .await
            .unwrap();

        BufReceiver::recv(&self.recv, id).await
    }
}

pub type Interrupt = Arc<Notify>;

struct Client {
    send: Sender<Rx>,
    intr: Interrupt,
    buffer: VecDeque<Tx>,
}

/// Serial muliplexer.
struct Mux {
    port: SerialStream,
    clients: HashMap<Addr, Client>,
    common_recv: PriorityReceiver<(Tx, Addr), Priority>,
    common_send: PrioritySender<(Tx, Addr), Priority>,
    max_client_dur: Duration,
}

impl Mux {
    pub fn new(port: SerialStream, max_client_dur: Duration) -> Self {
        let (common_send, common_recv) = priority_channel::<(Tx, Addr), Priority>();
        Self {
            port,
            common_recv,
            common_send,
            clients: HashMap::new(),
            max_client_dur,
        }
    }

    pub fn add_client(&mut self, addr: Addr) -> Option<Handle> {
        let vacant = match self.clients.entry(addr) {
            Entry::Vacant(vacant) => vacant,
            Entry::Occupied(..) => return None,
        };
        let (send, recv) = channel::<Rx>();
        let intr = Arc::new(Notify::new());
        vacant.insert(Client {
            send,
            intr: intr.clone(),
            buffer: VecDeque::new(),
        });
        Some(Handle {
            req: Requester::new(addr, self.common_send.clone(), recv),
            intr,
        })
    }

    pub async fn run(mut self) -> ! {
        let mut sched = Scheduler::new(self.clients.keys().copied(), self.max_client_dur);
        loop {
            let immediate = loop {
                match self.common_recv.try_recv() {
                    Ok(((tx, addr), priority)) => match priority {
                        Priority::Queued => {
                            self.clients.get_mut(&addr).unwrap().buffer.push_back(tx)
                        }
                        Priority::Immediate => break Some((tx, addr)),
                    },
                    Err(err) => {
                        if err.is_closed() {
                            panic!("{}", err);
                        }
                        break None;
                    }
                }
            };
            let queued = immediate.or_else(|| {
                let addr = sched.get_current();
                self.clients
                    .get_mut(&addr)
                    .unwrap()
                    .buffer
                    .pop_front()
                    .map(|tx| (tx, addr))
            });
        }
    }
}

struct Scheduler {
    queue: VecDeque<Addr>,
    max_dur: Duration,
    last_switch: Instant,
}

impl Scheduler {
    pub fn new<I: Iterator<Item = Addr>>(iter: I, max_dur: Duration) -> Self {
        Self {
            queue: VecDeque::from_iter(iter),
            max_dur,
            last_switch: Instant::now(),
        }
    }
    pub fn get_current(&mut self) -> Addr {
        let now = Instant::now();
        if now - self.last_switch > self.max_dur {
            let prev = self.queue.pop_front().unwrap();
            self.queue.push_back(prev);
            self.last_switch = now;
        }
        *self.queue.front().unwrap()
    }
}
