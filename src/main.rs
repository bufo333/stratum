mod app;
mod cli;
mod ingest;
mod model;
mod render;

#[tokio::main]
async fn main() {
    let cli = match cli::Cli::parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = app::run(cli).await {
        eprintln!("trmnlapp: {e:#}");
        std::process::exit(1);
    }
}
