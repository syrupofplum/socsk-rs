use std::error::Error;
use socks_lib;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    let server: socks_lib::Server = socks_lib::Server::new(socks_lib::Config::new("127.0.0.1", 1083));
    server.handle().await?;

    Ok(())
}
