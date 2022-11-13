use async_ringbuf::{AsyncConsumer, AsyncHeapRb, AsyncProducer};
use futures::{AsyncBufReadExt, AsyncWriteExt};
use std::{
    collections::HashMap,
    fmt::Debug,
    io,
    pin::Pin,
    str::FromStr,
    string::ToString,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::sleep,
};

use crate::serial::{Addr, LINE_TERM};

type Pipe = AsyncHeapRb<u8>;
type Writer = AsyncProducer<u8, Arc<Pipe>>;
type Reader = AsyncConsumer<u8, Arc<Pipe>>;

pub struct Emulator {
    writer: Writer,
    reader: Reader,
    devs: HashMap<Addr, Device>,
}

impl Emulator {
    pub fn new<I: Iterator<Item = Addr>>(addrs: I) -> (Self, SerialPort) {
        const LEN: usize = 32;
        let (fw, fr) = Pipe::new(LEN).split();
        let (bw, br) = Pipe::new(LEN).split();
        (
            Self {
                devs: addrs
                    .into_iter()
                    .map(|addr| (addr, Device::new(addr)))
                    .collect(),
                reader: fr,
                writer: bw,
            },
            SerialPort {
                writer: fw,
                reader: br,
            },
        )
    }

    async fn recv(&mut self, buf: &mut Vec<u8>) {
        loop {
            buf.clear();
            self.reader.read_until(LINE_TERM, buf).await.unwrap();
            assert!(buf.pop().unwrap() == LINE_TERM);
            if !buf.is_empty() {
                log::debug!("-> {}", String::from_utf8_lossy(buf));
                break;
            }
        }
    }

    async fn send(&mut self, msg: &[u8]) {
        log::debug!("<- {}", String::from_utf8_lossy(msg));
        self.writer.write_all(msg).await.unwrap();
        self.writer.write_all(&[LINE_TERM]).await.unwrap();
    }

    fn dev(&mut self, addr: Addr) -> &mut Device {
        self.devs.get_mut(&addr).unwrap()
    }

    pub async fn run(mut self) -> ! {
        let mut addr = None;
        let mut buf = Vec::new();
        loop {
            self.recv(&mut buf).await;
            let (name, args) = {
                let mut parts = buf.split(|b| *b == b' ');
                (parts.next().unwrap(), parts.collect::<Vec<_>>())
            };

            sleep(Duration::from_millis(100)).await;

            if name == b"ADR" {
                assert_eq!(args.len(), 1);
                addr = Some(parse_bytes(args[0]));
                assert!(self.devs.contains_key(addr.as_ref().unwrap()));
                self.send(b"OK").await;
            } else {
                let addr = *addr.as_ref().unwrap();
                match name {
                    b"IDN?" => {
                        self.send(b"TDK-Lambda Emulator").await;
                    }
                    b"SN?" => {
                        self.send(format!("Emu-{}", addr).as_bytes()).await;
                    }
                    b"PC" => {
                        self.dev(addr).current = parse_bytes(args[0]);
                        self.send(b"OK").await;
                    }
                    b"PC?" | b"MC?" => {
                        let value = self.dev(addr).current;
                        self.send(&to_bytes(value)).await;
                    }
                    b"PV" => {
                        self.dev(addr).voltage = parse_bytes(args[0]);
                        self.send(b"OK").await;
                    }
                    b"PV?" | b"MV?" => {
                        let value = self.dev(addr).voltage;
                        self.send(&to_bytes(value)).await;
                    }
                    _ => {
                        panic!("Unknown command {}", String::from_utf8_lossy(name));
                    }
                }
            }
        }
    }
}

pub fn parse_bytes<T: FromStr>(bytes: &[u8]) -> T
where
    T::Err: Debug,
{
    std::str::from_utf8(bytes).unwrap().parse().unwrap()
}

pub fn to_bytes<T: ToString>(x: T) -> Vec<u8> {
    x.to_string().into_bytes()
}

struct Device {
    addr: Addr,
    voltage: f64,
    current: f64,
}

impl Device {
    fn new(addr: Addr) -> Self {
        Self {
            addr,
            voltage: 0.0,
            current: 0.0,
        }
    }
}

pub struct SerialPort {
    writer: Writer,
    reader: Reader,
}

impl AsyncWrite for SerialPort {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.writer), cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.writer), cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.writer), cx)
    }
}
impl AsyncRead for SerialPort {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.reader), cx, buf)
    }
}
