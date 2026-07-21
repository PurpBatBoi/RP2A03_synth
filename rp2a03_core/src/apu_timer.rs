// rp2a03_core\src\apu_timer.rs
// Implementation based on the info from NESdev, Blaarg's "apu_ref" and Brad Taylor's "2A03 technical reference"

#[derive(Debug, Clone)]
pub struct ApuTimer {
    period: u16,
    counter: u16,
}

impl ApuTimer {
    pub fn new(period: u16) -> Self {
        Self {period, counter: period}
    }

    pub fn counter(&self) -> u16 {
        self.counter
    }

    pub fn set_period(&mut self, period: u16) {
        self.period = period;
    }

    pub fn period(&self) -> u16 {
        self.period
    }

    pub fn restart(&mut self) {
        self.counter = self.period;
    }

    pub fn clock(&mut self) -> bool {
        if self.counter > 0 {
            self.counter -= 1;
            false
        } else {
            self.restart();
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_starts_loaded() {
        let timer = ApuTimer::new(4);
        assert_eq!(timer.period(), 4);
        assert_eq!(timer.counter(),4);
    }

    #[test]
    fn clock_decrements_counter() {
        let mut timer = ApuTimer::new(4);
        let expired = timer.clock();

        assert!(!expired);
        assert_eq!(timer.counter(), 3);
    }

    #[test]
    fn timer_does_not_expire_early() {
        let mut timer = ApuTimer::new(4);

        for _ in 0..4{
            assert!(!timer.clock());
        }
    }

    #[test]
    fn timer_expires_after_period_plus_one_clocks() {
        let mut timer = ApuTimer::new(4);

        for _ in 0..4 {
            assert!(!timer.clock());
        }
        assert!(timer.clock());
    }

    #[test]
    fn timer_reloads_after_expiring() {
        let mut timer = ApuTimer::new(4);

        for _ in 0..4 {
            timer.clock();
        }

        assert!(timer.clock());

        assert_eq!(timer.counter(), 4);
    }

    #[test]
    fn changing_period_does_not_reload_counter() {
        let mut timer = ApuTimer::new(4);

        timer.clock();
        timer.clock();

        timer.set_period(10);

        assert_eq!(timer.counter(), 2);
        assert_eq!(timer.period(), 10);
    }

    #[test]
    fn reload_uses_new_period() {
        let mut timer = ApuTimer::new(4);

        timer.set_period(10);

        while !timer.clock() {}

        assert_eq!(timer.counter(), 10);
    }

    #[test]
    fn timer_output_sequence() {
        let mut timer = ApuTimer::new(4);

        let expected = [
            false,
            false,
            false,
            false,
            true,
            false,
            false,
        ];

        for expected_value in expected {
            assert_eq!(timer.clock(), expected_value);
        }
    }
}