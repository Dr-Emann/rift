use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "rift-http-proxy")]
#[command(about = "High-performance HTTP proxy")]
struct Args {
    #[arg(short, long, default_value = "8080")]
    port: u16,
    #[arg(short, long)]
    config: Option<String>,
}

fn main() {
    let args = Args::parse();
    println!("Starting on port {}", args.port);
}
