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

pub struct Engine {
}

impl Engine {
    fn new() -> Engine {
        Engine{}
    }
    
    #[inline]
    fn run_engine_one() {
    }
}
