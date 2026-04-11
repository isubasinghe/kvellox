use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use kvell_rs::spsc::new_slab_ring_buffer;

#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Entry {
    values: [u32; 16], // 16 × 4 = 64 bytes = 1 cache line
}

const ENTRY_SIZE: usize = std::mem::size_of::<Entry>();
const _: () = assert!(ENTRY_SIZE == 64);

const TOTAL_BYTES: u64 = 1024 * 1024 * 1024; // 1 GB
const ITEMS: usize = TOTAL_BYTES as usize / ENTRY_SIZE;

fn spsc_throughput(c: &mut Criterion) {
    let cores = core_affinity::get_core_ids().expect("failed to get core ids");
    assert!(cores.len() >= 2, "need at least 2 cores");
    let core_producer = cores[0];
    let core_consumer = cores[1];

    let mut group = c.benchmark_group("spsc");
    group.throughput(Throughput::Bytes(ITEMS as u64 * ENTRY_SIZE as u64));

    group.bench_function("intercore_cacheline_transfer", |b| {
        b.iter(|| {
            // 512 cache-line slots = 8 pages
            let (mut writer, mut reader) = new_slab_ring_buffer(512, ENTRY_SIZE).unwrap();

            let cp = core_producer;
            let producer = std::thread::spawn(move || {
                core_affinity::set_for_current(cp);
                let mut counter: u32 = 0;
                for _ in 0..ITEMS {
                    loop {
                        let ok = writer.try_write_item(|ptr| {
                            let entry = ptr as *mut Entry;
                            let mut values = [0u32; 16];
                            for v in &mut values {
                                *v = counter;
                                counter += 1;
                            }
                            unsafe { entry.write(Entry { values }) };
                            true
                        });
                        if ok {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                }
            });

            let cc = core_consumer;
            let consumer = std::thread::spawn(move || {
                core_affinity::set_for_current(cc);
                let mut checksum: u32 = 0;
                for _ in 0..ITEMS {
                    loop {
                        let ok = reader.try_read_item(|ptr| {
                            let entry = unsafe { (ptr as *const Entry).read() };
                            for &v in &entry.values {
                                checksum = checksum.wrapping_add(v);
                            }
                            true
                        });
                        if ok {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                }
                std::hint::black_box(checksum);
            });

            producer.join().unwrap();
            consumer.join().unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, spsc_throughput);
criterion_main!(benches);
