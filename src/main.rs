use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

struct PunterTransfer {
    payload: Vec<u8>,
}

impl PunterTransfer {
    async fn wait_send_block<R: Unpin + AsyncReadExt>(&self, mut read: R) -> io::Result<R> {
        loop {
            let x = read.read_u8().await?;
            if x != 'S' as u8 {
                continue;
            }
            let x = read.read_u8().await?;
            if x != '/' as u8 {
                continue;
            }
            let x = read.read_u8().await?;
            if x != 'B' as u8 {
                continue;
            }
            return Ok(read);
        }
    }

    async fn wait_good<R: Unpin + AsyncRead>(&mut self, mut read: R) -> io::Result<R> {
        loop {
            let x = read.read_u8().await?;
            if x != 'G' as u8 {
                continue;
            }
            let x = read.read_u8().await?;
            if x != 'O' as u8 {
                continue;
            }
            let x = read.read_u8().await?;
            if x != 'O' as u8 {
                continue;
            }
            return Ok(read);
        }
    }

    async fn send_block<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        unimplemented!();
    }

    async fn send_ack<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        write.write_all(&"ACK".as_bytes()).await?;
        Ok(write)
    }

    async fn send_syn<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        write.write_all(&"SYN".as_bytes()).await?;
        Ok(write)
    }

    async fn transfer<R: AsyncReadExt + Unpin, W: AsyncWrite + Unpin>(
        &mut self,
        read: R,
        write: W,
    ) -> io::Result<(R, W)> {
        let mut r = Some(read);
        let mut w = Some(write);

        loop {
            let read = self.wait_send_block(r.take().unwrap()).await?;
            let write = self.send_block(w.take().unwrap()).await?;
            let read = self.wait_good(read).await?;
            let write = self.send_ack(write).await?;

            r = Some(read);
            w = Some(write);

            if self.payload.len() == 0 {
                break;
            }
        }

        let read = self.wait_send_block(r.take().unwrap()).await?;
        let write = self.send_syn(w.take().unwrap()).await?;
        let read = self.wait_send_block(read).await?;

        Ok((read, write))
    }
}

async fn handle_client(mut conn: TcpStream) -> io::Result<()> {
    let (read, mut write) = conn.split();
    let mut bufread = BufReader::new(read);
    let mut transfer = None;

    loop {
        let mut line = vec![128];

        bufread.read_until('\r' as u8, &mut line).await?;

        println!("{:?}", line);
        let strline = std::str::from_utf8(&line).expect("UTF decode");
        println!("{}", strline);
        write.write_all(&line).await?;
        write.write_all(&"\r".as_bytes()).await?;

        match &strline {
            &"BYE" => {
                break;
            }
            &"GET" => {
                write.write_all(&" WHICH FILE?\r".as_bytes()).await?;
                bufread.read_until('\r' as u8, &mut line).await?;
                let fname = std::str::from_utf8(&line).expect("UTF decode");
                transfer = Some(fname.to_string());
                break;
            }
            _ => {
                write.write_all(&" WAT?\r".as_bytes()).await?;
            }
        }
    }

    if let Some(fname) = transfer {
        println!("Transferring {}", fname);
        write.write_all(&" SENDING NOW\r".as_bytes()).await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let addr: SocketAddr = "0.0.0.0:6400".parse().unwrap();
    let mut listener = TcpListener::bind(&addr).await?;

    loop {
        let (c, _) = listener.accept().await?;
        handle_client(c).await?;
    }
}
