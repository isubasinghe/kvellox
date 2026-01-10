use super::cache::S3Fifo;

pub struct PageCacheManager<'a> {
    pages: S3Fifo<i64, Page<'a>>
}

pub struct Page<'a> {
    data: &'a[u8]
}

impl<'a> PageCacheManager<'a> {
    pub fn new(num_pages: usize) -> PageCacheManager<'a>  {
        let pages_cache = S3Fifo::new(num_pages);
        PageCacheManager { pages: pages_cache }
    }

    pub fn get_page(&mut self) -> Option<Page<'a>> {
        None
    }

    pub fn page_hit(&mut self, page_idx: i64) {
    }
}

