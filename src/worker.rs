use rart::{keys::vector_key::VectorKey, tree::AdaptiveRadixTree};
use crate::slab::Slab;
use std::path::Path;
use std::error::Error;
use futures::channel::oneshot;

pub enum Command {
    Write {
        key: Vec<u8>,
        value: Vec<u8>,
        resp: oneshot::Sender<Result<(), String>>, // Error must be Send
    }
}

pub struct Worker {
    index: AdaptiveRadixTree<VectorKey, (u64, u64)>,
    slab: Slab<256>,
}

impl Worker {
    pub async fn new(path: &Path) -> Result<Self, Box<dyn Error>> {
        let slab = Slab::new(path).await;
        Ok(Worker {
            index: AdaptiveRadixTree::new(),
            slab,
        })
    }

    pub async fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn Error>> {
        self.slab.write_item(key, value).await?;
        // TODO: Update Index
        Ok(())
    }
}
