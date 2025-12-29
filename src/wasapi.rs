use anyhow::Result;
use audioadapter_buffers::direct::InterleavedSlice;
use rtrb::{chunks::ChunkError, Consumer, Producer, RingBuffer};
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters};
use std::{
    sync::Arc,
    thread::{self, sleep_ms, yield_now},
};
use wasapi::*;
use windows::Win32::{
    Media::Audio::{
        AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_DEVICE_IN_USE,
        AUDCLNT_E_ENDPOINT_CREATE_FAILED, AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED,
        AUDCLNT_E_UNSUPPORTED_FORMAT,
    },
    System::Threading::{
        GetCurrentThread, SetThreadPriority, INFINITE, THREAD_PRIORITY_TIME_CRITICAL,
    },
};

use crate::ring::{new_framering, FrameRingConsumer};

use std::println as info;
use std::println as debug;
use std::println as warn;
use std::println as error;

/// Convert f32 samples to the hardware format (optimized)
fn convert_samples_to_bytes(
    samples: &[f32],
    byte_buffer: &mut Vec<u8>,
    bits_per_sample: u16,
    is_float: bool,
) {
    byte_buffer.clear();

    if bits_per_sample == 32 && is_float {
        // Fast path: direct memory copy for f32
        let bytes =
            unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 4) };
        byte_buffer.extend_from_slice(bytes);
    } else {
        match bits_per_sample {
            16 => {
                for &sample in samples {
                    let val = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    byte_buffer.extend_from_slice(&val.to_le_bytes());
                }
            }
            24 => {
                for &sample in samples {
                    let val = (sample.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
                    byte_buffer.extend_from_slice(&val.to_le_bytes()[..3]);
                }
            }
            32 => {
                for &sample in samples {
                    let val = (sample.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                    byte_buffer.extend_from_slice(&val.to_le_bytes());
                }
            }
            _ => unreachable!("Unsupported bit depth: {}", bits_per_sample),
        }
    }
}

pub fn start_wasapi(
    mut asio_consumer: FrameRingConsumer,
    sample_rate: usize,
    channels: usize,
) -> Result<()> {
    // WASAPI requires COM initialized on the calling thread
    let _ = initialize_mta();

    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator.get_default_device(&Direction::Render)?;
    info!("Using device: {}", device.get_friendlyname()?);

    let mut audio_client = device.get_iaudioclient()?;

    // Requested format (shared mode will autoconvert if needed)
    let hw_format = WaveFormat::new(
        32, // container bits
        32, // valid bits
        &SampleType::Int,
        sample_rate,
        channels,
        Some(0x3),
    );

    debug!("Requested format: {:?}", hw_format);

    // Query device timing and use minimum period for lowest latency
    let (_default_period, min_period) = audio_client.get_device_period()?;

    info!(
        "Device periods: min {} ({}ms)",
        min_period,
        min_period as f64 / 10_000.0
    );

    // Evented shared mode with minimum period
    let mode = StreamMode::EventsShared {
        autoconvert: false,
        buffer_duration_hns: min_period,
    };

    // Evented exclusive
    let mode = StreamMode::EventsExclusive {
        period_hns: min_period,
    };

    match audio_client.initialize_client(&hw_format, &Direction::Render, &mode) {
        Ok(()) => debug!("IAudioClient::Initialize ok"),
        Err(e) => {
            if let WasapiError::Windows(ref werr) = e {
                match werr.code() {
                    AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED => {
                        warn!("Buffer not aligned; retrying with aligned duration");
                        let buffer_frames = audio_client.get_buffer_size()?;
                        let aligned_period =
                            (buffer_frames as i64 * 10_000_000) / sample_rate as i64;

                        audio_client = device.get_iaudioclient()?;
                        let mode = StreamMode::EventsShared {
                            autoconvert: true,
                            buffer_duration_hns: aligned_period,
                        };
                        audio_client.initialize_client(&hw_format, &Direction::Render, &mode)?;
                    }
                    AUDCLNT_E_DEVICE_IN_USE => {
                        error!("Device already in use");
                        return Err(e.into());
                    }
                    AUDCLNT_E_UNSUPPORTED_FORMAT => {
                        error!("Unsupported audio format");
                        return Err(e.into());
                    }
                    AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => {
                        error!("Exclusive mode not allowed");
                        return Err(e.into());
                    }
                    AUDCLNT_E_ENDPOINT_CREATE_FAILED => {
                        error!("Endpoint creation failed");
                        return Err(e.into());
                    }
                    _ => {
                        error!("IAudioClient::Initialize failed: {:?}", e);
                        return Err(e.into());
                    }
                }
            } else {
                return Err(e.into());
            }
        }
    }

    let render_client = audio_client.get_audiorenderclient()?;
    let event_handle = audio_client.set_get_eventhandle()?;
    let buffer_frames = audio_client.get_buffer_size()? as usize;

    info!(
        "WASAPI buffer: {} frames ({}ms)",
        buffer_frames,
        buffer_frames as f64 / sample_rate as f64 * 1000.0
    );

    // Create rtrb ring buffer
    let (mut output_producer, mut consumer) = new_framering(channels, buffer_frames * 2, "wasapi");

    let input_rate = 48_000.0;
    let output_rate = sample_rate as f64;
    let resample_ratio = output_rate / input_rate; // 4.0
    let resample_chunk_size = 64;
    let mut resampler = Async::<f32>::new_sinc(
        resample_ratio,
        1.1, // Max ratio relative
        &SincInterpolationParameters {
            sinc_len: 64,
            f_cutoff: 0.95,
            oversampling_factor: 128,
            interpolation: rubato::SincInterpolationType::Cubic,
            window: rubato::WindowFunction::BlackmanHarris2,
        },
        resample_chunk_size, // Chunk size
        channels,            // Number of channels
        FixedAsync::Output,  // Fixed input size
    )?;

    // Set thread priority to time-critical for audio
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }

    // Pre-allocate buffers to avoid allocations in the render loop
    let mut sample_buffer = vec![0.0f32; buffer_frames * channels];
    let mut byte_buffer = Vec::<u8>::with_capacity(buffer_frames * channels);

    let bits_per_sample = hw_format.get_bitspersample();
    let is_float = hw_format.get_subformat().ok() == Some(SampleType::Float);

    audio_client.start_stream()?;
    info!("Audio stream started");

    // clear out asio buffer
    while asio_consumer.pop_into(1, vec![0.0].as_mut_slice()) > 0 {}

    // preallocated buffer, we need no more than the wasapi buffer size
    let mut staging = vec![0.0f32; buffer_frames * channels];

    // Spawn resampler thread
    thread::spawn(move || {
        use std::time::Instant;
        let mut last_log = Instant::now();
        loop {
            // Inside the loop, before the availability check
            if last_log.elapsed().as_millis() >= 1000 {
                info!(
                    "ASIO ring: {} frames available, WASAPI ring: {} frames available",
                    asio_consumer.available_frames(),
                    output_producer.usage()
                );
                last_log = Instant::now();
            }

            // Accumulate input samples
            let in_frames = resampler.input_frames_next();
            let in_samples = in_frames * channels;

            let staging = &mut staging[..in_samples];
            while asio_consumer.pop_into(in_frames, staging) == 0 {
                yield_now();
            }

            // Prepare input slice for resampler
            let input = match InterleavedSlice::new(staging, channels, in_frames) {
                Ok(i) => i,
                Err(e) => {
                    warn!("Failed to create input slice: {:?}", e);
                    continue;
                }
            };

            // Resample
            let output = match resampler.process(&input, 0, None) {
                Ok(o) => o.take_data(),
                Err(e) => {
                    warn!("Resampler error: {:?}", e);
                    continue;
                }
            };
            // Inside the loop, before the availability check
            if last_log.elapsed().as_millis() >= 1000 {
                info!(
                    "ASIO ring: {} frames available, WASAPI ring: {} frames available",
                    asio_consumer.available_frames(),
                    output_producer.usage()
                );
                last_log = Instant::now();
            }

            // Push resampled frames into rtrb ring (fast bulk operation!)
            if !output.is_empty() {
                output_producer.push(&output);
            }
        }
    });

    // clear out wasapi buffer
    while consumer.pop_into(1, vec![0.0].as_mut_slice()) > 0 {}

    // ===== Render loop =====
    loop {
        match event_handle.wait_for_event(INFINITE) {
            Ok(_) => Ok(()),
            Err(WasapiError::EventTimeout) => continue,
            Err(e) => Err(e),
        }?;

        let available_frames = match audio_client.get_available_space_in_frames() {
            Ok(frames) => frames as usize,
            Err(e) => {
                error!("Failed to get available frames: {:?}", e);
                continue;
            }
        };

        // Normal render path - pop samples from ring
        let sample_count = available_frames * channels;
        let frames_read = consumer.pop_into(available_frames, &mut sample_buffer[..sample_count]);

        // partial or 0 read
        if frames_read < available_frames {
            sample_buffer[frames_read * channels..sample_count].fill(0.0); // Fill remaining with silence
        }

        // Convert samples to hardware format
        convert_samples_to_bytes(
            &sample_buffer[..sample_count],
            &mut byte_buffer,
            bits_per_sample,
            is_float,
        );

        // Write to device
        if let Err(e) = render_client.write_to_device(available_frames, &byte_buffer, None) {
            error!("Failed to write to device: {:?}", e);
            continue;
        }
    }
}
