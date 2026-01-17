use sled;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open Sled DB
    let tree = sled::open("/tmp/sled_bench_db")?;

    println!("Sled DB opened. Starting benchmark...");
    
    let count = 10_000;
    let keys: Vec<String> = (0..count).map(|i| format!("key-{}", i)).collect();
    let vals: Vec<String> = (0..count).map(|i| format!("val-{}", i)).collect();

    let start = Instant::now();

    // Sled is thread-safe and synchronous (blocking) for inserts, 
    // but internally handles concurrency well.
    // For a fair comparison with our concurrent-dispatch benchmark, 
    // we could spawn threads, but Sled is often used simply in a loop 
    // because its internal WAL handles batching.
    // However, to stress it like we stressed Kvellox, let's just loop tight.
    // (Kvellox was async-concurrent, Sled is sync-concurrent capable).
    
    // Attempting parallel inserts using Rayon would be "fairest" for throughput,
    // but a tight loop is the baseline.
    
    for i in 0..count {
        tree.insert(keys[i].as_bytes(), vals[i].as_bytes())?;
    }
    
    // Explicit flush to ensure durability (similar to what Kvellox does implicitly via O_DIRECT/fsync equivalent)
    tree.flush()?;

    let duration = start.elapsed();
    println!("Sled Writes completed! {} items in {:?}", count, duration);
    println!("Throughput: {:.2} items/sec", count as f64 / duration.as_secs_f64());

    Ok(())
}
