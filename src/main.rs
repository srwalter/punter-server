use std::io;
use std::io::Read;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

// The largest expressible block size is 255, and that includes the header, which this number does not
const MAX_BLOCK_SIZE: u8 = 248;

struct PunterTransfer {
    payload: Vec<u8>,
    metadata_block: bool,
    block_num: u16,
    next_block_size: u8,
}

mod punter {
    use std::convert::TryInto;
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

        pub fn from_bytes(bytes: &[u8]) -> PunterHeader {
            assert_eq!(bytes.len(), 7);

            let check_add = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
            let check_xor = u16::from_le_bytes(bytes[2..4].try_into().unwrap());
            let block_size = bytes[4];
            let block_num = u16::from_le_bytes(bytes[5..7].try_into().unwrap());

            PunterHeader {
                check_add,
                check_xor,
                block_size,
                block_num,
            }
        }

        pub fn to_bytes(&self) -> Vec<u8> {
            let mut cursor = io::Cursor::new(vec![7]);
            cursor.write_all(&self.check_add.to_le_bytes()).unwrap();
            cursor.write_all(&self.check_xor.to_le_bytes()).unwrap();
            // Size includes this header, which is 7 bytes
            cursor.write_all(&[self.block_size + 7]).unwrap();
            cursor.write_all(&self.block_num.to_le_bytes()).unwrap();

            cursor.into_inner()
        }

        pub fn check_add(&self, payload: &[u8]) -> u16 {
            let mut sum: u16 = 0;
            let bytes = self.to_bytes();

            for b in &bytes {
                sum += *b as u16;
            }

            for b in payload {
                sum += *b as u16;
            }

            sum
        }

        pub fn check_xor(&self, payload: &[u8]) -> u16 {
            let mut sum: u16 = 0;
            let bytes = self.to_bytes();

            for b in bytes.iter().chain(payload) {
                sum ^= *b as u16;
                let high_bit = sum & 0x8000;
                sum <<= 1;
                if high_bit != 0 {
                    sum |= 1;
                }
            }

            sum
        }
    }
}

enum GoodBadSb {
    Good,
    Bad,
    Sb,
}

impl PunterTransfer {
    fn new(payload: Vec<u8>, metadata_block: bool) -> Self {
        let next_block_size = if metadata_block { 8 } else { 7 };
        PunterTransfer {
            payload,
            metadata_block,
            block_num: 0,
            next_block_size,
        }
    }

    async fn wait_word<R: Unpin + AsyncReadExt>(
        mut read: R,
        word: (char, char, char),
    ) -> io::Result<R> {
        loop {
            let x = read.read_u8().await?;
            println!("Got {}", x);
            if x != word.0 as u8 {
                continue;
            }
            let x = read.read_u8().await?;
            println!("Got {}", x);
            if x != word.1 as u8 {
                continue;
            }
            let x = read.read_u8().await?;
            println!("Got {}", x);
            if x != word.2 as u8 {
                continue;
            }
            println!("  Got it!");
            return Ok(read);
        }
    }

    async fn wait_block<R: Unpin + AsyncReadExt>(&mut self, mut read: R) -> io::Result<R> {
        println!("Waiting for block ({} bytes)", self.next_block_size);

        let mut buf = vec![0 as u8; self.next_block_size as usize];
        read.read_exact(&mut buf).await?;

        let header = &buf[0..7];
        let header = punter::PunterHeader::from_bytes(header);

        if self.next_block_size > 7 {
            self.payload.extend_from_slice(&buf[7..]);
        }

        self.next_block_size = if header.block_num & 0xff00 == 0xff || self.metadata_block {
            0
        } else {
            header.block_size
        };

        // XXX: verify checksum

        Ok(read)
    }

    async fn wait_send_block<R: Unpin + AsyncReadExt>(read: R) -> io::Result<R> {
        println!("Waiting for S/B");
        Self::wait_word(read, ('S', '/', 'B')).await
    }

    async fn wait_syn<R: Unpin + AsyncReadExt>(read: R) -> io::Result<R> {
        println!("Waiting for SYN");
        Self::wait_word(read, ('S', 'Y', 'N')).await
    }

    async fn wait_good<R: Unpin + AsyncRead>(read: R) -> io::Result<R> {
        println!("Waiting for GOO");
        Self::wait_word(read, ('G', 'O', 'O')).await
    }

    async fn wait_ack<R: Unpin + AsyncReadExt>(mut read: R) -> io::Result<(bool, R)> {
        println!("Waiting for ACK");

        let mut buf = vec![0 as u8; 3];
        let _ = tokio::time::timeout(Duration::from_secs(1), read.read_exact(&mut buf)).await;

        let success = buf == vec!['A' as u8, 'C' as u8, 'K' as u8];
        if success {
            println!("Got ACK");
        } else {
            println!("Didn't get it");
        }
        Ok((success, read))
    }

    async fn wait_rx_word<R: Unpin + AsyncReadExt>(mut read: R) -> io::Result<(GoodBadSb, R)> {
        println!("Waiting for GOO or BAD");

        loop {
            let mut buf = vec![0 as u8; 3];
            let _ = tokio::time::timeout(Duration::from_secs(1), read.read_exact(&mut buf)).await;

            if buf == vec!['G' as u8, 'O' as u8, 'O' as u8] {
                println!("Got GOO");
                return Ok((GoodBadSb::Good, read));
            } else if buf == vec!['B' as u8, 'A' as u8, 'D' as u8] {
                println!("Got BAD");
                return Ok((GoodBadSb::Bad, read));
            } else if buf == vec!['S' as u8, '/' as u8, 'B' as u8] {
                println!("Got S/B");
                return Ok((GoodBadSb::Sb, read));
            } else {
                println!("Didn't get good or bad {:?}", buf);
                let mut buf = Vec::new();
                let _ = tokio::time::timeout(Duration::from_millis(1), read.read_to_end(&mut buf))
                    .await;
            }
        }
    }

    async fn maybe_send_block<R: Unpin + AsyncReadExt, W: Unpin + AsyncWrite>(
        &self,
        read: R,
        mut write: W,
    ) -> io::Result<(R, W)> {
        let (good, read) = Self::wait_rx_word(read).await?;

        match good {
            GoodBadSb::Sb => {
                write = self.send_block(write).await?;
            }
            _ => (),
        }

        return Ok((read, write));
    }

    async fn maybe_send_ack<R: Unpin + AsyncRead, W: Unpin + AsyncWrite>(
        &mut self,
        read: R,
        mut write: W,
    ) -> io::Result<(R, W)> {
        let (good, read) = Self::wait_rx_word(read).await?;

        match good {
            GoodBadSb::Good => {
                // Packet acknowledged, so we can send different data next time
                let block_size = self.get_last_block_size();
                self.block_num += 1;
                self.payload = self.payload.split_off(block_size as usize);
                write = Self::send_ack(write).await?;
            }
            GoodBadSb::Bad => {
                write = Self::send_ack(write).await?;
            }
            GoodBadSb::Sb => (),
        }

        return Ok((read, write));
    }

    fn get_last_block_size(&self) -> u8 {
        if self.block_num == 0 && !self.metadata_block {
            // For non-metadata blocks, the first block is a bare header
            0
        } else if self.payload.len() > MAX_BLOCK_SIZE as usize {
            // Otherwise, send as much as we can
            MAX_BLOCK_SIZE as u8
        } else {
            self.payload.len() as u8
        }
    }

    fn get_next_block_size(&self) -> u8 {
        if self.payload.len() > MAX_BLOCK_SIZE as usize {
            if self.payload.len() - MAX_BLOCK_SIZE as usize > MAX_BLOCK_SIZE as usize {
                MAX_BLOCK_SIZE
            } else {
                (self.payload.len() - MAX_BLOCK_SIZE as usize) as u8
            }
        } else {
            0 as u8
        }
    }

    async fn send_block<W: Unpin + AsyncWrite>(&self, mut write: W) -> io::Result<W> {
        let mut block_num = self.block_num;
        let next_block_size = self.get_next_block_size();
        if next_block_size == 0 && !self.metadata_block {
            block_num |= 0xff00;
        }
        println!("Sending block size {}", next_block_size);

        let last_block_size = self.get_last_block_size();
        let payload = &self.payload[0..last_block_size as usize];
        println!("Sending payload len {}", payload.len());

        let mut header = punter::PunterHeader::new(block_num, next_block_size);
        let check_add = header.check_add(payload);
        let check_xor = header.check_xor(payload);
        header.check_add = check_add;
        header.check_xor = check_xor;
        let header_bytes = header.to_bytes();
        println!("Sending block {:?}", header_bytes);
        write.write_all(&header_bytes).await?;
        write.write_all(payload).await?;

        Ok(write)
    }

    async fn send_ack<W: Unpin + AsyncWrite>(mut write: W) -> io::Result<W> {
        println!("Sent ACK");
        write.write_all(&"ACK".as_bytes()).await?;
        Ok(write)
    }

    async fn send_goo<W: Unpin + AsyncWrite>(mut write: W) -> io::Result<W> {
        println!("Sent GOO");
        write.write_all(&"GOO".as_bytes()).await?;
        Ok(write)
    }

    async fn send_sb<W: Unpin + AsyncWrite>(mut write: W) -> io::Result<W> {
        println!("Sent S/B");
        write.write_all(&"S/B".as_bytes()).await?;
        Ok(write)
    }

    async fn send_syn<W: Unpin + AsyncWrite>(mut write: W) -> io::Result<W> {
        println!("Sent SYN");
        write.write_all(&"SYN".as_bytes()).await?;
        Ok(write)
    }

    async fn upload<R: AsyncReadExt + Unpin, W: AsyncWrite + Unpin>(
        &mut self,
        mut read: R,
        mut write: W,
    ) -> io::Result<(R, W)> {
        read = Self::wait_good(read).await?;
        write = Self::send_ack(write).await?;

        loop {
            let x = self.maybe_send_block(read, write).await?;
            read = x.0;
            write = x.1;

            let x = self.maybe_send_ack(read, write).await?;
            read = x.0;
            write = x.1;

            if self.payload.len() == 0 {
                break;
            }
        }

        read = Self::wait_send_block(read).await?;
        write = Self::send_syn(write).await?;
        read = Self::wait_syn(read).await?;
        write = Self::send_sb(write).await?;

        Ok((read, write))
    }

    async fn download<R: AsyncReadExt + Unpin, W: AsyncWrite + Unpin>(
        &mut self,
        mut read: R,
        mut write: W,
    ) -> io::Result<(R, W)> {
        if self.metadata_block {
            read = Self::wait_good(read).await?
        };

        loop {
            write = Self::send_goo(write).await?;
            let x = Self::wait_ack(read).await?;
            let success = x.0;
            read = x.1;

            if success {
                break;
            }
        }

        loop {
            write = Self::send_sb(write).await?;
            read = self.wait_block(read).await?;
            write = Self::send_goo(write).await?;
            let x = Self::wait_ack(read).await?;
            let success = x.0;
            read = x.1;
            assert!(success);

            if self.next_block_size < 7 {
                break;
            }
        }

        write = Self::send_sb(write).await?;
        read = Self::wait_syn(read).await?;
        write = Self::send_syn(write).await?;
        read = Self::wait_send_block(read).await?;
        read = Self::wait_send_block(read).await?;
        read = Self::wait_send_block(read).await?;

        Ok((read, write))
    }
}

async fn read_line<T: AsyncBufReadExt + Unpin>(mut read: T) -> io::Result<(T, String)> {
    let mut line = Vec::new();
    read.read_until('\r' as u8, &mut line).await?;
    if line[0] == 128 as u8 {
        line.remove(0);
    }
    // Remove CR
    line.pop();
    Ok((
        read,
        std::str::from_utf8(&line).expect("UTF decode").to_string(),
    ))
}

async fn handle_client(mut conn: TcpStream) -> io::Result<()> {
    let (read, mut write) = conn.split();
    let mut bufread = BufReader::new(read);
    let mut transfer = None;
    let mut download = false;

    loop {
        let (buf2, strline) = read_line(bufread).await?;
        bufread = buf2;
        println!("{}", strline);
        write.write_all(&strline.as_bytes()).await?;
        write.write_all(&"\r".as_bytes()).await?;

        match strline.as_ref() {
            "BYE" => {
                break;
            }
            "GET" => {
                write.write_all(&" WHICH FILE?\r".as_bytes()).await?;
                let (buf2, fname) = read_line(bufread).await?;
                bufread = buf2;

                write.write_all(&fname.as_bytes()).await?;
                write.write_all(&"\r".as_bytes()).await?;
                transfer = Some(fname);
                break;
            }
            "PUT" => {
                download = true;
                break;
            }
            _ => {
                write.write_all(&" WAT?\r".as_bytes()).await?;
            }
        }
    }

    if download {
        write.write_all(&" READY TO RECEIVE\r".as_bytes()).await?;

        let payload = vec![];
        let mut punter = PunterTransfer::new(payload, true);
        let (bufread, write) = punter.download(bufread, write).await?;
        println!("File type {:?}", punter.payload);

        let payload = vec![];
        let mut punter = PunterTransfer::new(payload, false);
        let (_bufread, _write) = punter.download(bufread, write).await?;
        println!("Received {} bytes", punter.payload.len());
    } else if let Some(fname) = transfer {
        let fname = fname.to_lowercase().to_string();

        println!("Transferring {}", fname);
        write.write_all(&" SENDING NOW\r".as_bytes()).await?;

        let mut f = std::fs::File::open(fname)?;

        let payload = vec![1 as u8];
        let mut punter = PunterTransfer::new(payload, true);
        let (bufread, write) = punter.upload(bufread, write).await?;

        let mut payload = Vec::new();
        f.read_to_end(&mut payload)?;
        let mut punter = PunterTransfer::new(payload, false);
        let (_bufread, _write) = punter.upload(bufread, write).await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let addr: SocketAddr = "0.0.0.0:6400".parse().unwrap();
    let mut listener = TcpListener::bind(&addr).await?;

    loop {
        let (c, _) = listener.accept().await?;
        tokio::spawn(handle_client(c));
    }
}
