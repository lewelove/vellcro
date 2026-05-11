use clap::{Parser, Subcommand};

mod discid;
mod manifest;
mod remote;
mod utils;

#[derive(Parser)]
#[command(name = "vellcro", version = "0.1.0", about = "Vellum curation utility")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Manifest {
        url: String,
        #[arg(long)]
        metadata: bool,
        #[arg(long)]
        mbid: bool,
        #[arg(long)]
        torrent: String,
        #[arg(long, default_value = "flac,wav")]
        tracks: String,
    },
    Discid,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Manifest { url, metadata, mbid, torrent, tracks } => {
            if let Err(e) = manifest::run(&url, metadata, mbid, &torrent, &tracks) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Discid => {
            discid::run();
        }
    }
}
