use etcd_client::{Client, Error};
use std::time::Instant;
use futures::future::join_all;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut client = Client::connect(["localhost:2379"], None).await?;

    println!("Connected to etcd. Starting benchmark...");
    
    // Generate data
    let count = 10_000;
    let keys: Vec<String> = (0..count).map(|i| format!("key-{}", i)).collect();
    let vals: Vec<String> = (0..count).map(|i| format!("val-{}", i)).collect();

    let start = Instant::now();

    // To avoid "too many open files" or connection limits, we might need to batch or semaphore.
    // But 10,000 futures on localhost might be okay if the client multiplexes on HTTP/2.
    // etcd-client multiplexes.
    
    let mut futures = Vec::with_capacity(count);
    
    for i in 0..count {
        let mut kv_client = client.kv_client();
        let key = keys[i].clone();
        let val = vals[i].clone();
        
        futures.push(tokio::spawn(async move {
            kv_client.put(key, val, None).await
        }));
    }

    let results = join_all(futures).await;
    
    // Check for errors
    for res in results {
        let _ = res.unwrap()?;
    }

    let duration = start.elapsed();
    println!("etcd Writes completed! {} items in {:?}", count, duration);
    println!("Throughput: {:.2} items/sec", count as f64 / duration.as_secs_f64());

    Ok(())
}
