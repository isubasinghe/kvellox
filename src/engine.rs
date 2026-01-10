use std::thread;
use tokio::runtime;
use std::sync::{Condvar, Arc, Mutex};
use super::worker::Worker;

const MaxArraySize: usize = 64;

pub struct Engine {
    num_workers: u32,
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
    pub async fn new(num_workers: u64) -> Engine {

        Engine{num_workers: 1}
    }
}
