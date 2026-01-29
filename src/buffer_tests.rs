//! Unit tests for the buffer module

#[cfg(test)]
mod tests {
    use crate::buffer::PlaybackBuffer;
    use crate::sources::Sample;

    #[test]
    fn test_playback_buffer_default() {
        let buffer = PlaybackBuffer::default();
        assert!(!buffer.is_eof());
        assert!(!buffer.has_data());
    }

    #[test]
    fn test_playback_buffer_push_and_read() {
        let mut buffer = PlaybackBuffer::default();

        let samples: Vec<Sample> = vec![(100, 100), (200, 200), (300, 300)];
        buffer.push_samples(samples);

        let pulled = buffer.pull_samples(3);
        assert_eq!(pulled, vec![(100, 100), (200, 200), (300, 300)]);
        // Buffer should be empty after reading all samples
        assert!(!buffer.has_data());
    }

    #[test]
    fn test_playback_buffer_clear() {
        let mut buffer = PlaybackBuffer::default();

        buffer.push_samples(vec![(1, 1), (2, 2), (3, 3)]);
        buffer.pull_samples(1); // Read one sample

        buffer.clear();

        assert!(!buffer.has_data());
        assert!(!buffer.is_eof());
    }

    #[test]
    fn test_playback_buffer_eof_flag() {
        let mut buffer = PlaybackBuffer::default();

        assert!(!buffer.is_eof());

        buffer.set_eof(true);
        assert!(buffer.is_eof());

        buffer.set_eof(false);
        assert!(!buffer.is_eof());

        // Clear should reset EOF flag
        buffer.set_eof(true);
        buffer.clear();
        assert!(!buffer.is_eof());
    }

    #[test]
    fn test_playback_buffer_paused() {
        let mut buffer = PlaybackBuffer::default();

        buffer.push_samples(vec![(100, 100), (200, 200)]);

        // When paused, should return silence (0, 0)
        buffer.set_paused(true);
        let pulled = buffer.pull_samples(2);
        assert_eq!(pulled, vec![(0, 0), (0, 0)]);

        // When unpaused, should resume from where it was
        buffer.set_paused(false);
        let pulled = buffer.pull_samples(1);
        assert_eq!(pulled, vec![(100, 100)]);
    }

    #[test]
    fn test_playback_buffer_position() {
        let mut buffer = PlaybackBuffer::default();

        // Position should be 0 initially
        assert_eq!(buffer.get_total_position_secs(44100), 0.0);

        // Add samples and read some
        buffer.push_samples(vec![(1, 1); 44100]); // 1 second of audio at 44100 Hz

        // Read half the samples
        buffer.pull_samples(22050);

        // Position should be approximately 0.5 seconds
        let position = buffer.get_total_position_secs(44100);
        assert!((position - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_playback_buffer_position_after_clear() {
        let mut buffer = PlaybackBuffer::default();

        buffer.push_samples(vec![(1, 1); 1000]);
        buffer.pull_samples(500);

        buffer.clear();

        // Position should reset to 0 after clear
        assert_eq!(buffer.get_total_position_secs(44100), 0.0);
    }

    #[test]
    fn test_playback_buffer_multiple_pushes() {
        let mut buffer = PlaybackBuffer::default();

        buffer.push_samples(vec![(1, 1), (2, 2)]);
        buffer.push_samples(vec![(3, 3), (4, 4)]);

        let pulled = buffer.pull_samples(4);
        assert_eq!(pulled, vec![(1, 1), (2, 2), (3, 3), (4, 4)]);
        assert!(!buffer.has_data());
    }

    #[test]
    fn test_playback_buffer_empty_push() {
        let mut buffer = PlaybackBuffer::default();

        buffer.push_samples(Vec::new());

        assert!(!buffer.has_data());
    }

    #[test]
    fn test_playback_buffer_pull_pads_with_silence() {
        let mut buffer = PlaybackBuffer::default();

        buffer.push_samples(vec![(100, 100), (200, 200)]);

        // Request more samples than available - should pad with silence
        let pulled = buffer.pull_samples(5);
        assert_eq!(
            pulled,
            vec![(100, 100), (200, 200), (0, 0), (0, 0), (0, 0)]
        );
    }
}
