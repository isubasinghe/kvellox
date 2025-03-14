use std::thread;
use crossbeam_queue::ArrayQueue;

const MaxArraySize: usize = 64;

pub struct Engine {
    num_workers: u32,
    workers: Vec<worker>
}

pub struct Transaction {
    tx_id: u64
}

pub struct worker {
}

fn xorhash(data: &[u8], sz: u64) -> u64 {
    const seed: u64 = 73802;
    let mut h:u64 = seed;

    for c in data {
        h = h ^ ( ( h << 5) + (*c as u64) + (h >> 2));
    }
    h%sz
}

impl Engine {
    pub fn new(num_workers: u32) -> Self {
        let workers = Vec::new();
        for i in (0..num_workers) {
            let hnd = thread::spawn(||{
            });
        }
        Engine{num_workers, workers}
    }
    
    // write key writes a key value pair along with a tombstone flag.
    // the tombstone flag is used for transactions
    pub fn write_key(&self, key: &[u8], value: &[u8], tx_id: Option<u64>) {
        let worker_id = xorhash(key, self.num_workers as u64);
    }

    pub fn read_key(key: &[u8]) {
    }

    pub fn start_tx() -> Transaction {
        todo!()
    }

    pub fn execute_tx(tx: Transaction) {
    }
}

