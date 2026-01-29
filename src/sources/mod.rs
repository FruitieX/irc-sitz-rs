//! Audio source abstraction for the new simplified audio pipeline.
//!
//! All audio sources implement the `AudioSource` trait, which provides
//! a pull-based interface for reading audio samples.

pub mod espeak;
pub mod symphonia;

/// A stereo sample pair (left, right) as 16-bit signed integers.
pub type Sample = (i16, i16);

/// Output sample rate for all audio (Discord native format).
pub const OUTPUT_SAMPLE_RATE: u32 = 48000;

/// Trait for audio sources that can produce samples on demand.
///
/// Sources may have different native sample rates but should resample
/// to `OUTPUT_SAMPLE_RATE` (48kHz) before returning samples.
pub trait AudioSource: Send {
    /// Pull up to `count` samples from the source.
    ///
    /// Returns:
    /// - `Some(samples)` with available samples (may be less than `count`)
    /// - `None` when the source is exhausted (EOF)
    ///
    /// If fewer samples are available than requested, returns what's available.
    /// Callers should check the returned vector length.
    fn pull_samples(&mut self, count: usize) -> Option<Vec<Sample>>;

    /// Check if the source has more samples available.
    fn has_more(&self) -> bool;
}
