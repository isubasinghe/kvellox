use crate::worker::{Worker, Command};
use crate::entry::EntryHeader;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::path::{Path, PathBuf};
use flume::Sender;
use futures::channel::oneshot;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Storage {
    worker_senders: Vec<Sender<Command>>,
    num_workers: usize,
}

impl Storage {
    pub fn new(num_workers: usize, db_root: &Path) -> Result<Storage, Box<dyn Error>> {
        let mut worker_senders = Vec::with_capacity(num_workers);
        
        for i in 0..num_workers {
            let worker_path = db_root.join(format!("worker-{}", i));
            if !worker_path.exists() {
                std::fs::create_dir_all(&worker_path)?;
            }

            let (tx, rx) = flume::unbounded();
            worker_senders.push(tx);

            let path_owned = worker_path.clone();
            
            // Spawn standard thread
            thread::spawn(move || {
                // Worker manages its own runtime now
                let mut worker = Worker::new(&path_owned).unwrap();
                worker.run(rx);
            });
        }
        
        Ok(Storage { worker_senders, num_workers })
    }

    fn hash_key(key: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    pub async fn write_item(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Box<dyn Error>> {
        let hash = Self::hash_key(&key);
        let worker_idx = (hash as usize) % self.num_workers;
        let (resp_tx, resp_rx) = oneshot::channel();
        
        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let header = EntryHeader {
            timestamp: time,
            key_size: key.len() as i64,
            value_size: value.len() as i64,
            meta: 0 
        };
        
        let mut blob = Vec::with_capacity(EntryHeader::SIZE + key.len() + value.len());
        blob.extend_from_slice(&header.to_bytes());
        blob.extend_from_slice(&key);
        blob.extend_from_slice(&value);

        let cmd = Command::Update {
            blob,
            key,
            resp: resp_tx,
        };

        self.worker_senders[worker_idx].send(cmd).map_err(|_| "Worker died")?;
        
        resp_rx.await.map_err(|_| "Response dropped")??;
        Ok(())
    }

    pub fn get_item<'a>(&self, key: &'a [u8]) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
        let hash = Self::hash_key(key);
        let worker_idx = (hash as usize) % self.num_workers;
        let (resp_tx, resp_rx) = oneshot::channel();
        
        let cmd = Command::Get {
            key: key.to_vec(),
            resp: resp_tx,
        };

        self.worker_senders[worker_idx].send(cmd).map_err(|_| "Worker died")?;
        
        // Blocking wait for response
        let res = futures::executor::block_on(resp_rx).map_err(|_| "Response dropped")??;
        Ok(res)
    }
}
