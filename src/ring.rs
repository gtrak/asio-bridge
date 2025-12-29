use rtrb::{chunks::ChunkError, Consumer, Producer, RingBuffer};
use util::*;

use crate::util;
/// Fast lock-free ring buffer using rtrb
pub struct FrameRingConsumer {
    channels: usize,
    consumer: Consumer<f32>,
    name: String,
}

pub struct FrameRingProducer {
    channels: usize,
    producer: Producer<f32>,
    name: String,
}

pub fn new_framering(
    channels: usize,
    capacity: usize,
    name: &str,
) -> (FrameRingProducer, FrameRingConsumer) {
    info!(
        "Creating {} ring buffer with capacity {} frames",
        name, capacity
    );
    let (producer, consumer) = RingBuffer::<f32>::new(capacity * channels);
    (
        FrameRingProducer::new(channels, producer, name),
        FrameRingConsumer::new(channels, consumer, name),
    )
}

impl FrameRingProducer {
    fn new(channels: usize, producer: Producer<f32>, name: &str) -> Self {
        Self {
            channels,
            producer,
            name: name.to_owned(),
        }
    }

    pub fn available_frames(&self) -> usize {
        self.producer.slots() / self.channels
    }

    pub fn usage(&self) -> usize {
        self.producer.buffer().capacity() / self.channels - self.producer.slots() / self.channels
    }

    pub fn push(&mut self, output: &[f32]) {
        let mut chunk = {
            let chunk = self.producer.write_chunk(output.len());
            if let Ok(chunk) = chunk {
                chunk
            } else {
                // write what we can
                let ChunkError::TooFewSlots(available) = chunk.unwrap_err();
                self.producer.write_chunk(available).unwrap()
            }
        };

        let (a, b) = chunk.as_mut_slices();

        a.copy_from_slice(&output[..a.len()]);
        b.copy_from_slice(&output[a.len()..(a.len() + b.len())]);
        chunk.commit_all();
    }
}

impl FrameRingConsumer {
    fn new(channels: usize, consumer: Consumer<f32>, name: &str) -> Self {
        Self {
            channels,
            consumer,
            name: name.to_owned(),
        }
    }

    pub fn available_frames(&self) -> usize {
        self.consumer.slots() / self.channels
    }

    pub fn pop_into(&mut self, frames: usize, out: &mut [f32]) -> usize {
        let samples_needed = frames * self.channels;

        if out.len() < samples_needed {
            return 0;
        }

        // Try to read exact amount
        match self.consumer.read_chunk(samples_needed) {
            Ok(chunk) => {
                let samples_read = chunk.len();
                let (a, b) = chunk.as_slices();
                out[..a.len()].copy_from_slice(a);
                out[a.len()..].copy_from_slice(b);
                chunk.commit_all();
                samples_read / self.channels
            }
            Err(_) => 0,
        }
    }
}
