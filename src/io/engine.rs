use std::collections::VecDeque;
use std::error::Error;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, RawFd};
use std::{io, ptr};

use io_uring::{opcode, squeue, types, IoUring, SubmissionQueue};

#[derive(Clone, Debug)]
enum Token {
    Accept,
    Poll {
        fd: RawFd,
    },
    Read {
        fd: RawFd,
        buf_index: usize,
    },
    Write {
        fd: RawFd,
        buf_index: usize,
        offset: usize,
        len: usize,
    },
}

pub struct AcceptCount {
    entry: squeue::Entry,
    count: usize,
}

impl AcceptCount {
    fn new(fd: RawFd, token: usize, count: usize) -> AcceptCount {
        AcceptCount {
            entry: opcode::Accept::new(types::Fd(fd), ptr::null_mut(), ptr::null_mut())
                .build()
                .user_data(token as _),
            count,
        }
    }

    pub fn push_to(&mut self, sq: &mut SubmissionQueue<'_>) {
        while self.count > 0 {
            unsafe {
                match sq.push(&self.entry) {
                    Ok(_) => self.count -= 1,
                    Err(_) => break,
                }
            }
        }

        sq.sync();
    }
}

pub enum WorkUnit {
    OpenFile(OpenFile),
    ReadFile(ReadFile),
    WriteFile(WriteFile),
}

pub struct OpenFile {
    
}


pub struct ReadFile {
}

pub struct WriteFile {

}

pub trait Task {
    fn submit(&self, token: u64);
    fn on_complete(self: Box<Self>, result: i32) -> Option<Box<dyn Task>>;
}

pub struct Engine {
    ring: IoUring,
}

impl Engine {
    pub fn new() -> Result<Engine, Box<dyn Error>> {
        let ring = IoUring::new(256)?;
        Ok(Engine{ring})
    }

    pub fn register_buffer(&mut self) {
    }
    
    #[inline]
    fn run_engine_one() {
    }
    pub fn submit_task(&mut self, task: Box<dyn Task>) {
    }
}

impl Task for ReadFile {
    fn submit(&self, token: u64) {
        
    }
    fn on_complete(self: Box<Self>, result: i32) -> Option<Box<dyn Task>> {
        return None
    }
}
