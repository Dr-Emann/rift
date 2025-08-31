use clap::Parser;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use rift_http_proxy::proxy::ProxyServer;

#[derive(Parser, Debug)]
#[command(name = "rift-http-proxy")]
struct Args {
    #[arg(short, long, default_value = "8080")]
    port: u16,
    #[arg(short, long)]
    config: Option<String>,
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let level = if args.verbose { Level::DEBUG } else { Level::INFO };
    let subscriber = FmtSubscriber::builder().with_max_level(level).finish();
    tracing::subscriber::set_global_default(subscriber).ok();

    info!("Starting Rift on port {}", args.port);

    let server = ProxyServer::new("0.0.0.0", args.port);
    if let Err(e) = server.run().await {
        tracing::error!("Server error: {}", e);
    }
}
