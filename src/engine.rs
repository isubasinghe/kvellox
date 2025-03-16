use std::thread;
use tokio::runtime;
use std::sync::{Condvar, Arc, Mutex};

const MaxArraySize: usize = 64;

pub struct Engine {
    num_workers: u32,
    workers: Vec<Arc<(Mutex<bool>, Condvar)>>
}

pub struct Transaction {
    tx_id: u64,
}

fn xorhash(data: &[u8], sz: u64) -> u64 {
    const seed: u64 = 73802;
    let mut h: u64 = seed;

    for c in data {
        h = h ^ ((h << 5) + (*c as u64) + (h >> 2));
    }
    h % sz
}

pub enum Action {
    Request,
    Response,
    Die,
    Wait,
}

impl Engine {
    pub fn new(num_workers: u32) -> Self {
        let mut workers = Vec::new();
        for i in (0..num_workers) {
            let pair = Arc::new((Mutex::new(false), Condvar::new()));
            let pref = pair.clone();
            workers.push(pref);
            thread::spawn(move || {
                let rt = runtime::Builder::new_current_thread().build().unwrap();
                loop {
                    let (value, cvar) = &*pair;
                    let mut guard = value.lock().unwrap();
                    while !*guard {
                        guard = cvar.wait(guard).unwrap();
                    }
                }
            });
        }
        Engine { num_workers, workers}
    }

    // write key writes a key value pair along with a tombstone flag.
    // the tombstone flag is used for transactions
    pub fn write_key(&self, key: &[u8], value: &[u8], tx_id: Option<u64>) {
        let worker_id = xorhash(key, self.num_workers as u64);
    }

    pub fn read_key(key: &[u8]) {}

    pub fn start_tx() -> Transaction {
        todo!()
    }

    pub fn execute_tx(tx: Transaction) {}
}
