//! Simple ring buffer for audio samples.
//!
//! Used by audio decoders to push samples and by the mixer to pull them.

use crate::sources::Sample;

/// Threshold for compacting buffer - when read position exceeds this, we shift data.
/// At 48kHz, 48000 samples = 1 second worth of consumed audio.
const COMPACT_THRESHOLD: usize = 48000;

#[derive(Default)]
pub struct PlaybackBuffer {
    position: usize,
    buffer: Vec<Sample>,
    eof: bool,
    paused: bool,
    /// Total samples consumed since last clear() - for progress tracking
    total_samples_consumed: usize,
}

impl PlaybackBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.position = 0;
        self.buffer.clear();
        self.eof = false;
        self.total_samples_consumed = 0;
    }

    /// Compact the buffer by removing already-consumed samples
    fn compact(&mut self) {
        if self.position > 0 {
            self.buffer.drain(..self.position);
            self.position = 0;
        }
    }

    /// Read up to `count` samples at once, padding with silence if not enough available.
    pub fn pull_samples(&mut self, count: usize) -> Vec<Sample> {
        if self.paused {
            return vec![(0, 0); count];
        }

        let available = self.buffer.len().saturating_sub(self.position);
        let to_read = count.min(available);

        let mut samples = Vec::with_capacity(count);

        if to_read > 0 {
            samples.extend_from_slice(&self.buffer[self.position..self.position + to_read]);
            self.position += to_read;
            self.total_samples_consumed += to_read;
        }

        // Pad with silence if not enough samples
        samples.resize(count, (0, 0));

        // Compact periodically to prevent unbounded growth
        if self.position >= COMPACT_THRESHOLD {
            self.compact();
        }

        samples
    }

    /// Get total playback position in seconds (survives buffer chunk clears)
    pub fn get_total_position_secs(&self, sample_rate: u32) -> f64 {
        self.total_samples_consumed as f64 / sample_rate as f64
    }

    pub fn push_samples<I: IntoIterator<Item = Sample>>(&mut self, samples: I) {
        self.buffer.extend(samples);
    }

    pub fn is_eof(&self) -> bool {
        self.eof
    }

    pub fn set_eof(&mut self, eof: bool) {
        self.eof = eof;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Check if buffer has audio data available
    pub fn has_data(&self) -> bool {
        self.position < self.buffer.len()
    }

    /// Get current buffer level in samples (for diagnostics)
    pub fn buffer_level(&self) -> usize {
        self.buffer.len().saturating_sub(self.position)
    }
}
