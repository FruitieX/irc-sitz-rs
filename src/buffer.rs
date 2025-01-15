use crate::mixer::Sample;

#[derive(Default)]
pub struct PlaybackBuffer {
    position: usize,
    buffer: Vec<Sample>,
    eof: bool,
    paused: bool,
}

impl PlaybackBuffer {
    pub fn clear(&mut self) {
        self.position = 0;
        self.buffer.clear();
        self.eof = false;
    }

    pub fn next_sample(&mut self) -> Option<Sample> {
        if self.paused {
            return Some((0, 0));
        }

        let sample = self.buffer.get(self.position).cloned();
        self.position += 1;
        if self.position >= self.buffer.len() {
            self.position = 0;
            self.buffer.clear();
        }
        sample
    }

    pub fn get_position_secs(&self, sample_rate: u32) -> f64 {
        self.position as f64 / sample_rate as f64
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
}
