mod asio;
mod ring;
mod util;
mod visualizer;
mod wasapi;

use std::sync::Arc;
use visualizer::AudioVisualizer;

fn main() -> anyhow::Result<()> {
    // needs to be big enough to hold bursty asio buffers
    let visualizer = Arc::new(AudioVisualizer::new());

    // Start the visualizer
    visualizer.start();

    // Set the visualizer for ASIO
    asio::set_visualizer(visualizer.clone());

    // largest asio buffer
    let (asio_producer, asio_consumer) = ring::new_framering(2, 2048, "asio");

    unsafe {
        asio::start_asio(asio_producer)?;
    }

    wasapi::start_wasapi(asio_consumer, 192000, 2)?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
