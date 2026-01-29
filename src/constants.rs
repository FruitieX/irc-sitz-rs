// Define some constants for the audio parameters
pub const SAMPLE_RATE: u32 = 48000; // 48 kHz sample rate (Discord native format)
pub const BIT_DEPTH: u16 = 16; // 16 bits per sample
pub const CHANNELS: u16 = 2; // Stereo channel

// espeak-ng generates audio at 22050 Hz
pub const ESPEAK_SAMPLE_RATE: u32 = 22050;
