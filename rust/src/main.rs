use clap::{Parser, Subcommand};

mod collection;
mod config;
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
        mb_url: String,
        #[arg(long)]
        flags: Option<String>,
        #[arg(long, conflicts_with = "disk")]
        torrent: Option<String>,
        #[arg(
            long, 
            conflicts_with = "torrent", 
            num_args = 0..=1, 
            default_missing_value = "."
        )]
        disk: Option<String>,
        #[arg(
            long, 
            num_args = 0..=1, 
        )]
        tracks: String,
    },
    Discid,
    Collection {
        #[command(subcommand)]
        command: CollectionCommands,
    }
}

#[derive(Subcommand)]
enum CollectionCommands {
    Add {
        url: String,
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Manifest { mb_url, flags, torrent, disk, tracks } => {
            if let Err(e) = manifest::run(&mb_url, flags.as_deref(), torrent.as_deref(), disk.as_deref(), &tracks) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Discid => {
            discid::run();
        }
        Commands::Collection { command } => match command {
            CollectionCommands::Add { url } => {
                if let Err(e) = collection::run_add(&url) {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
