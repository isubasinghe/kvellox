use libc;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::io::engine::*;

pub struct Slab {
    engine: Engine
}

impl Slab {
    pub fn new(path: &Path, slab_id: u64, slab_size: u64) -> Slab {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        options.custom_flags(libc::O_DIRECT);

        let file_name = path.join(format!("db-size-{}.dbpart", slab_size));
        let std_file = options.open(file_name).unwrap();
        let meta = std_file.metadata().unwrap();

        let engine = Engine::new();
        todo!()
    }

    pub fn rebuild(&mut self) {

        let num_pages = 12;

        for i in 0..num_pages {
            self.engine.submit_task(Box::new(ReadFile{}));
        }
    }

    pub fn read_page(&mut self, page_idx: u64) {}

    pub fn write_page(&mut self) {}
}
