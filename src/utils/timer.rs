use std::time::{Duration, Instant};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Timer {
    start: Instant,
    accumulated_time: Duration,
}

impl Timer {
    pub fn now() -> Self {
        Self {
            start: Instant::now(),
            accumulated_time: Duration::new(0, 0),
        }
    }

    pub fn pause(&mut self) {
        let duration = self.start.elapsed();
        self.accumulated_time += duration;
    }

    pub fn start(&mut self) {
        self.start = Instant::now();
    }

    pub fn duration_label(&self) -> String {
        let duration = self.start.elapsed() + self.accumulated_time;
        if duration.as_secs() > 0 {
            format!("{:.2}s", duration.as_secs_f64())
        } else if duration.as_millis() > 0 {
            format!("{}ms", duration.as_millis())
        } else {
            format!("{}µs", duration.as_micros())
        }
    }
}
