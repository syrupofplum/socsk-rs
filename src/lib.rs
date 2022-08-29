use std::io::{Error, ErrorKind};
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::JoinHandle;

type PortType = u16;

type Byte = u8;

const VERSION: Byte = 5;

type AddressType = Byte;
const ATYP_IPV4: AddressType = 1;
const ATYP_DOMAIN_NAME: AddressType = 3;
const ATYP_IPV6: AddressType = 4;

type CmdType = Byte;
const CMD_CONNECT: CmdType = 1;
const CMD_ASSOCIATE: CmdType = 3;

const READER_BUFFER_LEN: usize = 256;

#[derive(Debug)]
pub struct Config {
    local_addr: String,
    local_port: PortType,
}

struct Address {
    addr: String,
    port: PortType,
    atyp: AddressType,
}

impl Config {
    pub fn new<S: Into<String>>(local_addr: S, local_port: u16) -> Self {
        Config {
            local_addr: local_addr.into(),
            local_port,
        }
    }
}

pub struct Server {
    config: Config,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Server {
            config
        }
    }

    pub async fn handle(&self) -> Result<(), Error> {
        let server_socket: TcpListener = TcpListener::bind((self.config.local_addr.as_str(), self.config.local_port)).await?;
        while let Ok((client_stream, _client_addr)) = server_socket.accept().await {
            tokio::spawn(async {
                let (client_reader, client_writer) = client_stream.into_split();
                let mut read_task: JoinHandle<Result<(), Error>> = tokio::spawn(async {
                    if let Err(err) = handle_connection(client_reader, client_writer).await {
                        return Err(err);
                    }
                    Ok(())
                });
                if tokio::try_join!(&mut read_task).is_err() {
                    eprintln!("err");
                }
            });
        }
        Ok::<(), Error>(())
    }
}

async fn handle_connection(mut client_reader: OwnedReadHalf, mut client_writer: OwnedWriteHalf) -> Result<(), Error> {
    let mut reader_buffer: [u8; READER_BUFFER_LEN] = [0u8; READER_BUFFER_LEN];

    let ver = client_reader.read_u8().await?;
    if VERSION != ver {
        return Err(Error::new(ErrorKind::InvalidInput, format!("invalid socks version {}", ver)));
    }
    let n_method = client_reader.read_u8().await?;
    let method_len = client_reader.read(&mut reader_buffer[..n_method as usize]).await?;
    if n_method as usize != method_len {
        return Err(Error::new(ErrorKind::InvalidInput, format!("invalid methods length {}", method_len)));
    }
    client_writer.write_all(&[5u8, 0u8]).await?;

    let ver = client_reader.read_u8().await?;
    if VERSION != ver {
        return Err(Error::new(ErrorKind::InvalidInput, format!("invalid socks version {}", ver)));
    }
    let cmd = client_reader.read_u8().await?;
    let _rsv = client_reader.read_u8().await?;

    let dst_addr = handle_connection_addr(&mut client_reader, &mut reader_buffer).await?;

    handle_connection_down(cmd, dst_addr, client_reader, client_writer).await?;

    Ok(())
}

async fn handle_connection_addr(client_reader: &mut OwnedReadHalf, reader_buffer: &mut [u8; 256]) -> Result<Address, Error> {
    let atyp = client_reader.read_u8().await?;
    let mut dst_addr: String;
    match atyp {
        ATYP_IPV4 => {
            let _ = client_reader.read(&mut reader_buffer[..4]).await?;
            dst_addr = String::from_utf8_lossy(&reader_buffer[..4]).to_string();
        }
        ATYP_DOMAIN_NAME => {
            let dst_addr_len: u8 = client_reader.read_u8().await?;
            if dst_addr_len as usize > 8192 {
                return Err(Error::new(ErrorKind::InvalidInput, format!("invalid dst_addr_len {}", dst_addr_len)));
            }
            let mut dst_addr_len_count = dst_addr_len as usize;
            dst_addr = String::with_capacity(dst_addr_len_count);
            while dst_addr_len_count != 0 {
                if dst_addr_len_count > READER_BUFFER_LEN {
                    let _ = client_reader.read(&mut reader_buffer[..]).await?;
                    dst_addr.push_str(String::from_utf8_lossy(&reader_buffer[..]).as_ref());
                    dst_addr_len_count -= READER_BUFFER_LEN;
                } else {
                    let _ = client_reader.read(&mut reader_buffer[..dst_addr_len_count]).await?;
                    dst_addr.push_str(String::from_utf8_lossy(&reader_buffer[..dst_addr_len_count]).as_ref());
                    dst_addr_len_count = 0;
                }
            }
        }
        ATYP_IPV6 => {
            let _ = client_reader.read(&mut reader_buffer[..16]).await?;
            dst_addr = String::from_utf8_lossy(&reader_buffer[..16]).to_string();
        }
        _ => {
            return Err(Error::new(ErrorKind::InvalidInput, format!("invalid atyp value {}", atyp)));
        }
    }
    let dst_port = client_reader.read_u16().await?;
    Ok(Address {
        addr: dst_addr,
        port: dst_port,
        atyp,
    })
}

async fn handle_connection_down(cmd: u8, dst_addr: Address, mut client_reader: OwnedReadHalf, mut client_writer: OwnedWriteHalf) -> Result<(), Error> {
    match cmd {
        CMD_CONNECT => {
            let (mut remote_reader, mut remote_writer) = handle_connect_tcp(dst_addr).await?;
            client_writer.write_all(&[5u8, 0u8, 0u8, 1u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8]).await?;

            let mut task_upstream: JoinHandle<Result<(),
                Error>> = tokio::spawn(async move {
                tokio::io::copy(&mut client_reader, &mut remote_writer).await?;
                Ok(())
            });

            let mut task_downstream: JoinHandle<Result<(),
                Error>> = tokio::spawn(async move {
                tokio::io::copy(&mut remote_reader, &mut client_writer).await?;
                Ok(())
            });

            tokio::try_join!(&mut task_upstream, &mut task_downstream)?;
        }
        CMD_ASSOCIATE => {
            let mut remote_socket = handle_connect_udp(dst_addr).await?;
            // todo associate implement
            return Err(Error::new(ErrorKind::InvalidInput, format!("unimplemented cmd value {}", cmd)));

        }
        _ => {
            return Err(Error::new(ErrorKind::InvalidInput, format!("invalid cmd value {}", cmd)));
        }
    }

    Ok(())
}

async fn handle_connect_tcp(dst_addr: Address) -> Result<(OwnedReadHalf, OwnedWriteHalf), Error> {
    let remote_stream = TcpStream::connect((dst_addr.addr.as_str(), dst_addr.port)).await?;
    let (remote_reader, remote_writer) = remote_stream.into_split();
    Ok((remote_reader, remote_writer))
}

async fn handle_connect_udp(dst_addr: Address) -> Result<UdpSocket, Error> {
    let remote_socket = UdpSocket::bind("0.0.0.0:0").await?;
    Ok(remote_socket)
}
