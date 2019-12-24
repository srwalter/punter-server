use std::io;
use std::net::SocketAddr;

use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

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
