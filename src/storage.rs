use crate::worker::{Worker, Command};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::path::{Path, PathBuf};
use flume::Sender;
use futures::channel::oneshot;
use std::thread;

pub struct Storage {
    worker_senders: Vec<Sender<Command>>,
    num_workers: usize,
}

impl Storage {
    pub async fn new(num_workers: usize, db_root: &Path) -> Result<Storage, Box<dyn Error>> {
        let mut worker_senders = Vec::with_capacity(num_workers);
        
        for i in 0..num_workers {
            let worker_path = db_root.join(format!("worker-{}", i));
            if !worker_path.exists() {
                std::fs::create_dir_all(&worker_path)?;
            }

            let (tx, rx) = flume::unbounded();
            worker_senders.push(tx);

            let path_owned = worker_path.clone(); // Move to thread
            
            // Spawn Worker Thread
            thread::spawn(move || {
                // Initialize Monoio Runtime on this thread
                let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
                    .enable_timer()
                    .build()
                    .unwrap();

                rt.block_on(async move {
                    let mut worker = Worker::new(&path_owned).await.unwrap(); // Handle error better?
                    
                    // Event Loop
                    while let Ok(cmd) = rx.recv_async().await {
                        match cmd {
                            Command::Write { key, value, resp } => {
                                let res = worker.write(&key, &value).await;
                                let res_string = res.map_err(|e| e.to_string());
                                let _ = resp.send(res_string);
                            }
                        }
                    }
                });
            });
        }
        
        Ok(Storage { worker_senders, num_workers })
    }

    fn hash_key(key: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    pub async fn write_item<'a>(&self, key: &'a [u8], value: &'a [u8]) -> Result<(), Box<dyn Error>> {
        let hash = Self::hash_key(key);
        let worker_idx = (hash as usize) % self.num_workers;
        
        let (resp_tx, resp_rx) = oneshot::channel();
        
        let cmd = Command::Write {
            key: key.to_vec(),
            value: value.to_vec(),
            resp: resp_tx,
        };

        // Send to worker (non-blocking if unbounded, or async wait)
        self.worker_senders[worker_idx].send_async(cmd).await.map_err(|_| "Worker died")?;
        
        // Wait for response
        let res = resp_rx.await.map_err(|_| "Response dropped")??;
        
        Ok(())
    }
}
