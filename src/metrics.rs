use sysinfo::System;
use std::time::{Duration, Instant};
use crate::protocol::HostMetrics;

pub fn run() {
    let mut sampler = Sampler::new();

    loop {
        std::thread::sleep(Duration::from_secs(1));

        let metrics = sampler.sample();
        match metrics.cpu_percent {
            Some(cpu) => println!("CPU: {:.1}%", cpu),
            None => println!("CPU: warming up"),
        }
        let used_gib = metrics.used_mem as f64 / 1_073_741_824.0;
        let total_gib = metrics.total_mem as f64 / 1_073_741_824.0;
        println!("RAM: {:.1} / {:.1} GiB", used_gib, total_gib);
        println!();
    }
}

pub struct Sampler {
    system: System,
    last_refresh: Instant,
    cpu_ready: bool,
}

impl Sampler {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu_usage();
        system.refresh_memory();

        Self { system, last_refresh: Instant::now(), cpu_ready: false }
    }

    pub fn sample(&mut self) -> HostMetrics {
        if self.last_refresh.elapsed() >= Duration::from_millis(1000) {
            self.system.refresh_cpu_usage();
            self.system.refresh_memory();

            self.cpu_ready = true;
            self.last_refresh = Instant::now();
        }

        HostMetrics {
            cpu_percent: if self.cpu_ready {
                Some(self.system.global_cpu_usage())
            }
            else {
                None
            },
            used_mem: self.system.used_memory(),
            total_mem: self.system.total_memory(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_memory_and_a_cpu_baseline() {
        let mut sampler = Sampler::new();
        let metrics = sampler.sample();
        assert!(metrics.cpu_percent.is_none());
        assert!(metrics.total_mem > 0);
        assert!(metrics.used_mem <= metrics.total_mem);
    }

    #[test]
    fn refreshes_cpu_then_reuses_the_cached_readings() {
        let mut sampler = Sampler::new();
        let before = sampler.last_refresh;
        std::thread::sleep(Duration::from_millis(1100));
        let metrics = sampler.sample();
        let refreshed_at = sampler.last_refresh;
        let cpu = metrics.cpu_percent.expect("CPU should be ready after the first interval");
        assert!(cpu.is_finite() && (0.0..=100.0).contains(&cpu));
        assert!(refreshed_at > before);
        let cached = sampler.sample();
        assert_eq!(sampler.last_refresh, refreshed_at);
        assert_eq!(cached.cpu_percent, metrics.cpu_percent);
        assert_eq!(cached.used_mem, metrics.used_mem);
        assert_eq!(cached.total_mem, metrics.total_mem);
    }
}
