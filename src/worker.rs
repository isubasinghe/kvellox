use std::collections::BTreeMap;
use crate::slab::{Slab, process_batch};
use std::path::Path;
use std::error::Error;
use futures::channel::oneshot;
use monoio::Runtime;
use monoio::time::TimeDriver;
use monoio::IoUringDriver;
use std::rc::Rc;
use std::cell::RefCell;
use futures::{select, FutureExt};
use futures::future::Either;

pub enum Command {
    Get {
        key: Vec<u8>,
        resp: oneshot::Sender<Result<Option<Vec<u8>>, String>>,
    },
    Update {
        blob: Vec<u8>,
        key: Vec<u8>,
        resp: oneshot::Sender<Result<(), String>>,
    }
}

pub struct Worker {
    // Replaced AdaptiveRadixTree with BTreeMap for stability
    index: BTreeMap<Vec<u8>, (u64, u64)>,
    slab: Rc<RefCell<Slab<256>>>,
    rt: Runtime<TimeDriver<IoUringDriver>>,
}

impl Worker {
    pub fn new(path: &Path) -> Result<Self, Box<dyn Error>> {
        let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
            .enable_timer()
            .build()
            .unwrap();

        let (slab, initial_data) = rt.block_on(async {
            let mut s = Slab::new(path).await;
            let data = s.rebuild().await;
            (s, data)
        });

        let mut index = BTreeMap::new();
        for (key, idx) in initial_data {
            index.insert(key, (idx as u64, 0));
        }
        
        Ok(Worker { index, slab: Rc::new(RefCell::new(slab)), rt })
    }

        fn process_command(cmd: Command, slab: &mut Slab<256>, index: &mut BTreeMap<Vec<u8>, (u64, u64)>) {
        match cmd {
            Command::Get { key, resp } => {
                if let Some((idx, _)) = index.get(&key) {
                    slab.queue_read(*idx as usize, resp);
                } else {
                    let _ = resp.send(Ok(None));
                }
            },
            Command::Update { blob, key, resp } => {
                let idx = slab.queue_write(blob, resp);
                if let Some((old_idx, _)) = index.insert(key, (idx as u64, 0)) {
                     slab.queue_tombstone(old_idx as usize);
                     slab.return_slot(old_idx as usize);
                }
            }
        }
    }

    pub fn run(&mut self, rx: flume::Receiver<Command>) {
        const BATCH_SIZE: usize = 256;
        const MIN_BATCH_SIZE: usize = 64;
        
        self.rt.block_on(async {
            let mut current_flush: Option<futures::future::Fuse<monoio::task::JoinHandle<()>>> = None;
            
            loop {
                let mut next_cmd = rx.recv_async().fuse();
                let mut flush_fut = if let Some(ref mut f) = current_flush {
                    Either::Left(f)
                } else {
                    Either::Right(futures::future::pending())
                };
                let mut timeout_fut = Box::pin(monoio::time::sleep(std::time::Duration::from_millis(1)).fuse());
                let mut force_flush = false;

                select! {
                    cmd_res = next_cmd => {
                        match cmd_res {
                            Ok(cmd) => {
                                let mut slab = self.slab.borrow_mut();
                                Self::process_command(cmd, &mut slab, &mut self.index);
                                
                                let mut gathered = 1;
                                while gathered < BATCH_SIZE {
                                    match rx.try_recv() {
                                        Ok(c) => {
                                            Self::process_command(c, &mut slab, &mut self.index);
                                            gathered += 1;
                                        },
                                        Err(_) => break,
                                    }
                                }
                            },
                            Err(_) => break,
                        }
                    },
                    _ = flush_fut => {
                        current_flush = None;
                    },
                    _ = timeout_fut => {
                        force_flush = true;
                    }
                }

                if current_flush.is_none() {
                    let mut slab = self.slab.borrow_mut();
                    let count = slab.pending_ops_count();
                    if count >= MIN_BATCH_SIZE || (force_flush && count > 0) {
                        let ops = slab.drain_ops();
                        let slab_ref = self.slab.clone();
                        current_flush = Some(monoio::spawn(process_batch(slab_ref, ops)).fuse());
                    }
                }
            }
        });
    }
}