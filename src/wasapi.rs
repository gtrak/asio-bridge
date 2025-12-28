use crate::AudioRing;
use cpal::{traits::*, Sample};
use std::sync::Arc;

pub fn start_wasapi(ring: Arc<AudioRing>, sample_rate: u32, channels: u16) -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();

    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            let mut offset = 0;

            while offset < data.len() {
                match ring.pop() {
                    Some(buf) => {
                        let n = buf.len().min(data.len() - offset);
                        data[offset..offset + n].copy_from_slice(&buf[..n]);
                        offset += n;
                    }
                    None => {
                        for x in &mut data[offset..] {
                            *x = 0.0;
                        }
                        break;
                    }
                }
            }
        },
        |err| eprintln!("cpal error: {err}"),
        None,
    )?;

    stream.play()?;
    std::mem::forget(stream);
    Ok(())
}
