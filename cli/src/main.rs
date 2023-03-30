use std::path::PathBuf;

use clap::Parser;

/// The commandline argument parser.
#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Args {
    /// The selected subcommand.
    #[command(subcommand)]
    command: Subcommands,
}

/// The subcommands for the CLI argument parser.
#[derive(clap::Subcommand)]
enum Subcommands {
    /// Execute an interaction combinator graph.
    Run {
        /// The path to the sucre file to execute.
        file: PathBuf
    },
}

fn main() {
    let args = Args::parse();

    let runtime = sucre::Runtime::new();
}
