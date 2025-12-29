use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct AudioVisualizer {
    is_running: Arc<AtomicBool>,
    last_update: std::sync::Mutex<Instant>,
    max_amplitude: std::sync::Mutex<f32>,
}

impl AudioVisualizer {
    pub fn new() -> Self {
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            last_update: std::sync::Mutex::new(Instant::now()),
            max_amplitude: std::sync::Mutex::new(0.0),
        }
    }

    pub fn start(&self) {
        self.is_running.store(true, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.is_running.store(false, Ordering::Relaxed);
    }

    pub fn update_amplitude(&self, amplitude: f32) {
        if !self.is_running.load(Ordering::Relaxed) {
            return;
        }

        // Update max amplitude
        let mut max_amp = self.max_amplitude.lock().unwrap();
        if amplitude > *max_amp {
            *max_amp = amplitude;
        }

        // Only update display every 100ms to avoid overwhelming console
        let now = Instant::now();
        let mut last_update = self.last_update.lock().unwrap();
        if now.duration_since(*last_update) > Duration::from_millis(100) {
            *last_update = now;
            self.display_visualization(amplitude, *max_amp);
            *max_amp = 0.0; // Reset max for next cycle
        }
    }

    fn display_visualization(&self, current_amplitude: f32, max_amplitude: f32) {
        // Create a simple bar visualization
        let bar_length = 30;
        let current_level = (current_amplitude * bar_length as f32).min(bar_length as f32) as usize;
        let max_level = (max_amplitude * bar_length as f32).min(bar_length as f32) as usize;

        let mut bar = String::with_capacity(bar_length + 20);
        bar.push('[');

        for i in 0..bar_length {
            if i < current_level {
                bar.push('|');
            } else if i < max_level {
                bar.push('█');
            } else {
                bar.push(' ');
            }
        }

        bar.push(']');
        bar.push_str(&format!(" {:.2} dB", 20.0 * current_amplitude.log10()));

        // Clear line and print new visualization
        print!("\r{}", bar);
        std::io::stdout().flush().unwrap();
    }
}

impl Drop for AudioVisualizer {
    fn drop(&mut self) {
        print!("\r");
        std::io::stdout().flush().unwrap();
    }
}
