use std::io;
use std::io::Read;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

struct PunterTransfer {
    payload: Vec<u8>,
    metadata_block: bool,
    block_num: u16,
}

mod punter {
    use std::io;
    use std::io::prelude::*;

    pub struct PunterHeader {
        pub check_add: u16,
        pub check_xor: u16,
        pub block_size: u8,
        pub block_num: u16,
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
    fn new(payload: Vec<u8>, metadata_block: bool) -> Self {
        PunterTransfer {
            payload,
            metadata_block,
            block_num: 0,
        }
    }

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

            let block_size = self.get_block_size();
            self.block_num += 1;
            self.payload = self.payload.split_off(block_size as usize);

            return Ok(read);
        }
    }

    fn get_block_size(&self) -> u8 {
        if self.block_num == 0 && !self.metadata_block {
            // For non-metadata blocks, the first block is a bare header
            0
        } else if self.payload.len() > 255 {
            // Otherwise, send as much as we can
            255 as u8
        } else {
            self.payload.len() as u8
        }
    }

    async fn send_block<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        let block_size = self.get_block_size();

        let mut header = punter::PunterHeader::new(self.block_num, block_size);
        let check_add = header.check_add();
        let check_xor = header.check_xor();
        header.check_add = check_add;
        header.check_xor = check_xor;
        let header_bytes = header.to_bytes();
        write.write_all(&header_bytes).await?;
        write
            .write_all(&self.payload[0..block_size as usize])
            .await?;

        Ok(write)
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
        let fname = fname.to_lowercase().to_string();

        println!("Transferring {}", fname);
        write.write_all(&" SENDING NOW\r".as_bytes()).await?;

        let mut f = std::fs::File::open(fname)?;

        let payload = vec!['1' as u8];
        let mut punter = PunterTransfer::new(payload, true);
        let (bufread, write) = punter.transfer(bufread, write).await?;

        let mut payload = Vec::new();
        f.read_to_end(&mut payload)?;
        let mut punter = PunterTransfer::new(payload, false);
        let (bufread, write) = punter.transfer(bufread, write).await?;
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
