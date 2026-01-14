use slab::Slab;
use std::path::Path;

mod cli;
mod page;
mod engine;
mod storage;
mod index;
mod io;
mod worker;
mod slab;
mod static_data;
mod entry;
mod cache;
mod fastcell;

use monoio;

fn main() {
    let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new().build().unwrap();

    let mut slab = Slab::<256>::new(Path::new("./db"));

    rt.block_on(async {
        let mut slab = slab.await;
        slab.write_item("hello".as_bytes(), "world".as_bytes()).await.unwrap() 
    });
    println!("Hello, world!");
}
