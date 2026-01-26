use rekv::rekvd::run_server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    // Run the rekvd server
    run_server().await?;

    Ok(())
}
