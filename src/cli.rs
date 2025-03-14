use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct CliOpts {
    num_workers: u32,
    page_size: u32,
    db: PathBuf
}
