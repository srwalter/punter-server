use std::io;
use std::net::SocketAddr;

use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

async fn handle_client(mut conn: TcpStream) -> io::Result<()> {
    let (read, mut write) = conn.split();
    let bufread = BufReader::new(read);
    let mut l = bufread.split('\r' as u8);
    loop {
        if let Some(line) = l.next_segment().await? {
            println!("{:?}", line);
            write.write_all(&line).await?;
        } else {
            break;
        }
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
