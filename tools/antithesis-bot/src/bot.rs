use tokio::net::TcpStream;

async fn launch(ip: String) -> eyre::Result<()> {
    println!("Connecting to {}", ip);

    // Connect to the TCP server
    let stream = TcpStream::connect(ip).await?;
    println!("Connected successfully!");

    // Keep the connection alive
    loop {
        // TODO: Implement message handling
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
