use std::io;
use std::net::SocketAddr;

use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

async fn handle_client(mut conn: TcpStream) {
    let (read, write) = conn.split();
    let bufread = BufReader::new(read);
    let mut l = bufread.lines();
    loop {
        if let Ok(maybe_line) = l.next_line().await {
            if let Some(line) = maybe_line {
                println!("{}", line);
            } else {
                break;
            }
        } else {
            break;
        }
    }
    ()
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let addr: SocketAddr = "0.0.0.0:6400".parse().unwrap();
    let mut listener = TcpListener::bind(&addr).await?;

    loop {
        let (c, _) = listener.accept().await?;
        handle_client(c).await;
    }
}
