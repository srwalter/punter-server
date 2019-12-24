use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

struct PunterTransfer {
    payload: Vec<u8>,
    first_block: bool,
    metadata_block: bool,
}

mod punter {
    use std::io;
    use std::io::prelude::*;

    pub struct PunterHeader {
        check_add: u16,
        check_xor: u16,
        block_size: u8,
        block_num: u16,
    }

    impl PunterHeader {
        pub fn new(block_num: u16, block_size: u8) -> PunterHeader {
            PunterHeader {
                check_add: 0,
                check_xor: 0,
                block_size,
                block_num,
            }
        }

        pub fn to_bytes(&self) -> Vec<u8> {
            let mut cursor = io::Cursor::new(vec![7]);
            cursor.write_all(&self.check_add.to_le_bytes()).unwrap();
            cursor.write_all(&self.check_xor.to_le_bytes()).unwrap();
            cursor.write_all(&[self.block_size]).unwrap();
            cursor.write_all(&self.block_num.to_le_bytes()).unwrap();

            cursor.into_inner()
        }

        pub fn check_add(&self) -> u16 {
            let mut sum: u16 = 0;
            let bytes = self.to_bytes();

            for b in &bytes {
                sum += *b as u16;
            }

            sum
        }

        pub fn check_xor(&self) -> u16 {
            let mut sum: u16 = 0;
            let bytes = self.to_bytes();

            for b in &bytes {
                sum ^= *b as u16;
                let high_bit = 0x8000;
                sum <<= 1;
                if high_bit != 0 {
                    sum |= 1;
                }
            }

            sum
        }
    }
}

impl PunterTransfer {
    async fn wait_send_block<R: Unpin + AsyncReadExt>(&self, mut read: R) -> io::Result<R> {
        println!("Waiting for S/B");
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
            println!("Got S/B");
            return Ok(read);
        }
    }

    async fn wait_good<R: Unpin + AsyncRead>(&mut self, mut read: R) -> io::Result<R> {
        println!("Waiting for GOO");
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
            println!("Got GOO");
            return Ok(read);
        }
    }

    async fn send_block<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        unimplemented!();
    }

    async fn send_ack<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        println!("Sent ACK");
        write.write_all(&"ACK".as_bytes()).await?;
        Ok(write)
    }

    async fn send_syn<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        println!("Sent SYN");
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
