use super::*;
use pin_project::pin_project;
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf},
    sync::mpsc::UnboundedSender as Sender,
    time::{sleep, timeout},
};

fn byte_is_intr(b: u8) -> Option<Addr> {
    if b >= 0x80 {
        Some(b - 0x80)
    } else {
        None
    }
}

pub struct AddrConnection<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> {
    conn: Connection<W, R>,
    active: Option<Addr>,
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> AddrConnection<W, R> {
    pub fn new((reader, writer): (R, W), intr: Sender<Addr>) -> Self {
        Self {
            conn: Connection::new((reader, writer), intr),
            active: None,
        }
    }

    pub async fn request(&mut self, addr: Addr, cmd: &str) -> Result<String, Error> {
        // Switch active address if needed
        if !self.active.map(|a| addr == a).unwrap_or(false) {
            self.conn
                .request(&format!("ADR {}", addr))
                .await
                .and_then(|resp| {
                    if resp == "OK" {
                        Ok(())
                    } else {
                        Err(Error::Device(resp))
                    }
                })?;
            self.active.replace(addr);
        }

        // Execute command
        self.conn.request(cmd).await.map_err(|err| {
            self.active.take();
            err
        })
    }

    pub async fn is_online(&mut self, addr: Addr) -> Result<bool, Error> {
        self.active.take();
        match self.conn.request(&format!("ADR {}", addr)).await {
            Ok(resp) => {
                if resp == "OK" {
                    self.active.replace(addr);
                    Ok(true)
                } else {
                    Err(Error::Device(resp))
                }
            }
            Err(Error::Timeout) => Ok(false),
            Err(err) => Err(err),
        }
    }
}

pub struct Connection<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> {
    writer: W,
    reader: BufReader<FilterReader<R>>,
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> Connection<W, R> {
    pub fn new((reader, writer): (R, W), intr: Sender<Addr>) -> Self {
        Self {
            writer,
            reader: BufReader::new(FilterReader::new(reader, intr)),
        }
    }

    pub async fn request(&mut self, cmd: &str) -> Result<String, Error> {
        for i in 0..CMD_RETRIES {
            sleep(CMD_DELAY).await;

            let mut buf = Vec::new();
            match timeout(CMD_TIMEOUT, async {
                clear(&mut self.reader).await?;

                self.writer.write_all(cmd.as_bytes()).await?;
                self.writer.write_u8(LINE_TERM).await?;
                self.writer.flush().await?;
                log::trace!("-> '{}'", cmd);

                buf.clear();
                self.reader.read_until(LINE_TERM, &mut buf).await?;
                if buf.pop().map(|b| b != LINE_TERM).unwrap_or(true) {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Serial connection closed unexpectedly",
                    ));
                }
                Ok(())
            })
            .await
            {
                Ok(io_res) => {
                    io_res?;
                    let resp = String::from_utf8(buf)?;
                    log::trace!("<- '{}'", resp);
                    return Ok(resp);
                }
                Err(_) => {
                    log::trace!("No response to '{}' (attempt: {})", cmd, i + 1);
                }
            }
        }
        Err(Error::Timeout)
    }
}

fn clear<R: AsyncBufRead + Unpin>(reader: &mut R) -> Clear<'_, R> {
    Clear { reader }
}

struct Clear<'a, R: AsyncBufRead + Unpin> {
    reader: &'a mut R,
}

impl<'a, R: AsyncBufRead + Unpin> Unpin for Clear<'a, R> {}

impl<'a, R: AsyncBufRead + Unpin> Future for Clear<'a, R> {
    type Output = Result<usize, io::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut count = 0;
        loop {
            let len = match Pin::new(&mut self.reader).poll_fill_buf(cx) {
                Poll::Pending => break Poll::Ready(Ok(count)),
                Poll::Ready(res) => match res {
                    Ok(buf) => {
                        log::error!("Unexpected input: {:?}", String::from_utf8_lossy(buf));
                        buf.len()
                    }
                    Err(err) => break Poll::Ready(Err(err)),
                },
            };
            Pin::new(&mut self.reader).consume(len);
            count += len;
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
                    match (this.prev.take(), byte_is_intr(b)) {
                        (Some(p), Some(a)) => {
                            if a == p {
                                this.chan.send(a).unwrap();
                            } else {
                                log::error!("SRQ bytes differ: {} != {}'", p, a);
                            }
                        }
                        (None, Some(a)) => {
                            this.prev.replace(a);
                        }
                        (op, None) => {
                            if let Some(p) = op {
                                log::error!("Single SRQ byte {}, two needed", p);
                            }
                            s[j] = b;
                            j += 1;
                        }
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
