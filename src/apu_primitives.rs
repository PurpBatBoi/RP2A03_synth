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

    pub fn clock(&mut self) -> bool {
        if self.counter > 0 {
            self.counter -= 1;
            false
        } else {
            self.counter = self.period;
            true
        }
    }
}