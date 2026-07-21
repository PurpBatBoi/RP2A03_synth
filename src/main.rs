mod apu_primitives;
use apu_primitives::ApuTimer;

fn main() {
    let mut timer = ApuTimer::new(4);

    for cycle in 0..10 {
        let expired = timer.clock();

        println!(
            "Cycle: {}: Counter ={}, Expired = {}",
            cycle,
            timer.counter(),
            expired
        );
    }
}
