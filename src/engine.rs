pub struct Engine {
}

pub struct Transaction {
    tx_id: u64
}

impl Engine {
    pub fn new() -> Self {
        Engine{}
    }
    
    // write key writes a key value pair along with a tombstone flag.
    // the tombstone flag is used for transactions
    pub fn write_key(key: &[u8], value: &[u8], tx_id: Option<u64>) {
    }

    pub fn read_key(key: &[u8]) {
    }

    pub fn start_tx() -> Transaction {
        todo!()
    }

    pub fn execute_tx(tx: Transaction) {
    }
}

