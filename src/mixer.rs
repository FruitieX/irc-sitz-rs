//! Pull-based audio mixer with automatic ducking.
//!
//! Mixes music and TTS audio sources, automatically ducking music volume
//! when TTS is playing. Uses smooth fade transitions for volume changes.

use crate::{
    buffer::PlaybackBuffer,
    sources::Sample,
};
use std::sync::{Arc, Mutex};

/// Number of audio samples per chunk (20ms at 48kHz = Discord Opus frame size)
pub const CHUNK_SIZE: usize = 960;

/// Volume multiplier for TTS (speech) - slightly louder than music
const TTS_VOLUME: f64 = 1.25;

/// Normal volume for music during playback
const MUSIC_VOLUME_NORMAL: f64 = 0.75;

/// Ducked volume for music when TTS is playing
const MUSIC_VOLUME_DUCKED: f64 = 0.2;

/// Samples to fade between volume levels (~150ms at 48kHz)
const FADE_SAMPLES: usize = 7200;

/// Volume change per sample during fade
const FADE_RATE: f64 = (MUSIC_VOLUME_NORMAL - MUSIC_VOLUME_DUCKED) / FADE_SAMPLES as f64;

/// Threshold to consider a sample "non-silent" for auto-duck detection
const SILENCE_THRESHOLD: i16 = 100;

/// Number of consecutive silent samples before unduck (~100ms at 48kHz)
const UNDUCK_SILENCE_SAMPLES: usize = 4800;

/// Shared buffer type
pub type SourceBuffer = Arc<Mutex<PlaybackBuffer>>;

/// Create a new shared source buffer
pub fn create_source_buffer() -> SourceBuffer {
    Arc::new(Mutex::new(PlaybackBuffer::new()))
}

/// Pull-based mixer that combines TTS and music sources with automatic ducking.
pub struct Mixer {
    /// TTS source buffer
    tts_buffer: SourceBuffer,
    /// Music source buffer
    music_buffer: SourceBuffer,
    /// Current music volume (smoothly interpolated)
    music_volume: f64,
    /// Target music volume (NORMAL or DUCKED)
    music_target_volume: f64,
    /// Count of consecutive silent TTS samples
    silence_count: usize,
}

impl Mixer {
    pub fn new(tts_buffer: SourceBuffer, music_buffer: SourceBuffer) -> Self {
        Self {
            tts_buffer,
            music_buffer,
            music_volume: MUSIC_VOLUME_NORMAL,
            music_target_volume: MUSIC_VOLUME_NORMAL,
            silence_count: UNDUCK_SILENCE_SAMPLES, // Start undocked
        }
    }

    /// Pull and mix samples from both sources.
    /// Called synchronously from Discord's audio thread.
    pub fn pull_samples(&mut self, count: usize) -> Vec<Sample> {
        // Pull from both sources
        let tts_samples = {
            let mut buf = self.tts_buffer.lock().unwrap();
            buf.pull_samples(count)
        };

        let music_samples = {
            let mut buf = self.music_buffer.lock().unwrap();
            buf.pull_samples(count)
        };

        // Mix the samples
        let mut output = Vec::with_capacity(count);

        for i in 0..count {
            let tts = tts_samples.get(i).copied().unwrap_or((0, 0));
            let music = music_samples.get(i).copied().unwrap_or((0, 0));

            // Auto-duck: check if TTS is producing sound
            let tts_has_audio = tts.0.abs() > SILENCE_THRESHOLD || tts.1.abs() > SILENCE_THRESHOLD;

            if tts_has_audio {
                self.silence_count = 0;
                self.music_target_volume = MUSIC_VOLUME_DUCKED;
            } else {
                self.silence_count += 1;
                if self.silence_count >= UNDUCK_SILENCE_SAMPLES {
                    self.music_target_volume = MUSIC_VOLUME_NORMAL;
                }
            }

            // Smooth volume transition
            if self.music_volume < self.music_target_volume {
                self.music_volume = (self.music_volume + FADE_RATE).min(self.music_target_volume);
            } else if self.music_volume > self.music_target_volume {
                self.music_volume = (self.music_volume - FADE_RATE).max(self.music_target_volume);
            }

            // Mix with volume scaling
            let left = (tts.0 as f64 * TTS_VOLUME + music.0 as f64 * self.music_volume) as i32;
            let right = (tts.1 as f64 * TTS_VOLUME + music.1 as f64 * self.music_volume) as i32;

            // Saturating conversion to i16
            let left = left.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let right = right.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

            output.push((left, right));
        }

        output
    }

    /// Get the current music volume (for diagnostics)
    #[allow(dead_code)]
    pub fn current_music_volume(&self) -> f64 {
        self.music_volume
    }

    /// Check if music is currently ducked
    #[allow(dead_code)]
    pub fn is_ducked(&self) -> bool {
        self.music_target_volume == MUSIC_VOLUME_DUCKED
    }
}

// Legacy MixerAction - kept for compatibility but most actions removed
#[derive(Clone, Debug)]
pub enum MixerAction {
    SetSecondaryChannelVolume(f64),
    #[allow(dead_code)]
    SetSecondaryChannelDuckedVolume(f64),
}

