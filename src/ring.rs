use crossbeam::queue::ArrayQueue;
use std::sync::Arc;

pub struct AudioRing {
    q: ArrayQueue<Vec<f32>>,
}

impl AudioRing {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            q: ArrayQueue::new(capacity),
        })
    }

    pub fn push(&self, buf: Vec<f32>) {
        let _ = self.q.push(buf);
    }

    pub fn pop(&self) -> Option<Vec<f32>> {
        self.q.pop()
    }
}
