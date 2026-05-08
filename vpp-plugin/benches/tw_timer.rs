use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use vpp_plugin::vppinfra::tw_timer::TimerWheel;

fn criterion_benchmark(c: &mut Criterion) {
    let mut tw = TimerWheel::<u8, 3, 64>::new();
    c.bench_function("add one and tick level 0", move |b| {
        b.iter(|| {
            let _ = tw.start_timer(8, 1);
            for _ in 0..8 {
                // Prevent inlining of tw.tick()
                black_box(());
                tw.expire_timers(1);
            }
        })
    });

    let mut tw = TimerWheel::<u8, 3, 64>::new();
    c.bench_function("add many and tick level 0", move |b| {
        b.iter(|| {
            for _ in 0..256 {
                let _ = tw.start_timer(8, 1);
            }
            for _ in 0..8 {
                // Prevent inlining of tw.tick()
                black_box(());
                tw.expire_timers(1);
            }
        })
    });

    let mut tw = TimerWheel::<u8, 3, 64>::new();
    c.bench_function("add one and tick level 1", move |b| {
        b.iter(|| {
            let _ = tw.start_timer(128, 1u8);
            for _ in 0..128 {
                // Prevent inlining of tw.tick()
                black_box(());
                tw.expire_timers(1);
            }
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
