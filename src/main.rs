mod asio;
mod ring;
mod wasapi;

use ring::AudioRing;

fn main() -> anyhow::Result<()> {
    let ring = AudioRing::new(64);

    unsafe {
        asio::start_asio(ring.clone())?;
    }

    wasapi::start_wasapi(ring, 48_000, 2)?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
