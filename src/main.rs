use storage::Storage;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use futures::stream::{FuturesUnordered, StreamExt};

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
    let db_path = Path::new("./db");
    
    // Ensure root exists
    if !db_path.exists() {
        std::fs::create_dir_all(db_path).unwrap();
    }

    // Wrap storage in Arc for multithreaded access
    let storage = Arc::new(Storage::new(4, db_path).unwrap());
    
    let num_client_threads = 4;
    println!("Starting writes (Sharded across 4 workers, {} client threads)...", num_client_threads);
    let start = std::time::Instant::now();
    
    let total_items = 1_000_000;
    let items_per_thread = total_items / num_client_threads;
    
    let mut handles = Vec::new();
    
    for t in 0..num_client_threads {
        let storage_ref = storage.clone();
        handles.push(thread::spawn(move || {
            let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
                   .build().unwrap();
            
            rt.block_on(async {
                let mut futures = FuturesUnordered::new();
                for i in 0..items_per_thread {
                    let unique_id = t * items_per_thread + i;
                    let key = format!("key-{}", unique_id).into_bytes();
                    let val = vec![0u8; 256]; // 256B value
                    
                    let s = storage_ref.clone();
                    futures.push(async move {
                        s.write_item(key, val).await
                    });

                    if futures.len() >= 1000 {
                        futures.next().await;
                    }
                }
                while let Some(_) = futures.next().await {}
            });
        }));
    }
    
    for h in handles {
        h.join().unwrap();
    }
    
    let duration = start.elapsed();
    let actual_total = items_per_thread * num_client_threads;
    println!("Writes completed! {} items in {:?}", actual_total, duration);
    println!("Throughput: {:.2} items/sec", actual_total as f64 / duration.as_secs_f64());
    
    println!("Hello, world!");
}
