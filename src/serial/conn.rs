use pin_project::pin_project;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf},
    sync::mpsc::UnboundedSender as Sender,
    time::{sleep, timeout},
};

use super::{Addr, Error, LINE_TERM};

fn byte_is_intr(b: u8) -> Option<Addr> {
    if b >= 0x80 {
        Some(b - 0x80)
    } else {
        None
    }
}

pub struct Connection<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> {
    writer: W,
    reader: BufReader<FilterReader<R>>,
    delay: Duration,
    timeout: Duration,
    retries: usize,
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> Connection<W, R> {
    pub fn new((reader, writer): (R, W), intr: Sender<Addr>) -> Self {
        Self {
            writer,
            reader: BufReader::new(FilterReader::new(reader, intr)),
            delay: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
            retries: 2,
        }
    }

    pub async fn request(&mut self, cmd: &str) -> Result<String, Error> {
        for i in 0..self.retries {
            sleep(self.delay).await;

            let mut buf = Vec::new();
            match timeout(self.timeout, async {
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
                    log::warn!("No response to '{}' (attempt: {})", cmd, i + 1);
                }
            }
        }
        Err(Error::Timeout)
    }
}

#[pin_project]
struct FilterReader<R: AsyncRead + Unpin> {
    #[pin]
    reader: R,
    prev: Option<Addr>,
    chan: Sender<Addr>,
}

impl<R: AsyncRead + Unpin> FilterReader<R> {
    pub fn new(reader: R, intr_chan: Sender<Addr>) -> Self {
        Self {
            reader,
            prev: None,
            chan: intr_chan,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for FilterReader<R> {
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
