use storage::Storage;
use std::path::Path;
use futures::future::join_all;

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

fn main() {
    let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
        .enable_timer()
        .build()
        .unwrap();

    let db_path = Path::new("./db");
    
    // Ensure root exists
    if !db_path.exists() {
        std::fs::create_dir_all(db_path).unwrap();
    }

    rt.block_on(async {
        let mut storage = Storage::new(4, db_path).await.unwrap();
        
        println!("Starting writes (Sharded across 4 workers)...");
        let start = std::time::Instant::now();
        
        let storage_ptr = &mut storage as *mut Storage;
        
        // Generate data and leak it to ensure 'static lifetime for the benchmark
        let keys: Vec<String> = (0..100_000).map(|i| format!("key-{}", i)).collect();
        let vals: Vec<String> = (0..100_000).map(|i| format!("val-{}", i)).collect();
        let keys_ref: &'static Vec<String> = Box::leak(Box::new(keys));
        let vals_ref: &'static Vec<String> = Box::leak(Box::new(vals));
        
        let mut futures = Vec::with_capacity(100_000);
        
        for i in 0..100_000 {
            unsafe {
                futures.push((*storage_ptr).write_item(keys_ref[i].as_bytes(), vals_ref[i].as_bytes()));
            }
        }
        
        // Execute batch
        let _results = join_all(futures).await;
        
        let duration = start.elapsed();
        println!("Writes completed! 100,000 items in {:?}", duration);
        println!("Throughput: {:.2} items/sec", 100_000.0 / duration.as_secs_f64());
    });
    println!("Hello, world!");
}
