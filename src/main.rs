mod asio;
mod ring;
mod visualizer;
mod wasapi;

use ring::AudioRing;
use std::sync::Arc;
use visualizer::AudioVisualizer;

fn main() -> anyhow::Result<()> {
    let ring = AudioRing::new(64);
    let visualizer = Arc::new(AudioVisualizer::new());

    // Start the visualizer
    visualizer.start();

    // Set the visualizer for ASIO
    asio::set_visualizer(visualizer.clone());

    unsafe {
        asio::start_asio(ring.clone())?;
    }

    wasapi::start_wasapi(ring, 48_000, 2)?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
