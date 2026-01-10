use slab::Slab;
use std::path::Path;

mod cli;
mod page;
mod engine;
mod storage;
mod index;
mod art;
mod io;
mod worker;
mod slab;
mod static_data;
mod entry;
mod cache;

use monoio;

fn main() {
    let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new().build().unwrap();

    let mut slab = Slab::<256>::new(Path::new("./db"));

    rt.block_on(slab);
    println!("Hello, world!");
}
