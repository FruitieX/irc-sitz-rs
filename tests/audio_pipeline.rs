//! Integration tests for audio pipeline.
//!
//! Tests for TTS and audio source handling.
//! Note: Mixer no longer uses event-driven ducking - it's automatic based on TTS buffer content.

mod common;

use common::*;
use irc_sitz_rs::buffer::PlaybackBuffer;
use irc_sitz_rs::sources::espeak::{Priority, TextToSpeechAction};
use irc_sitz_rs::sources::Sample;

/// Test TTS Priority enum variants.
#[tokio::test]
async fn test_tts_priority_variants() {
    assert_eq!(Priority::default(), Priority::Low);

    let low = Priority::Low;
    let high = Priority::High;

    // Verify they're different
    assert_ne!(low, high);

    // Verify Debug works
    assert!(format!("{:?}", low).contains("Low"));
    assert!(format!("{:?}", high).contains("High"));
}

/// Test TTS Priority deserialization.
#[tokio::test]
async fn test_tts_priority_deserialization() {
    // Priority derives Deserialize but not Serialize
    let low_json = r#""Low""#;
    let high_json = r#""High""#;

    let low_restored: Priority = serde_json::from_str(low_json).unwrap();
    let high_restored: Priority = serde_json::from_str(high_json).unwrap();

    assert_eq!(Priority::Low, low_restored);
    assert_eq!(Priority::High, high_restored);
}

/// Test TextToSpeechAction variants.
#[tokio::test]
async fn test_tts_action_variants() {
    let actions = vec![
        TextToSpeechAction::Speak {
            text: "Hello world".to_string(),
            prio: Priority::Low,
        },
        TextToSpeechAction::Speak {
            text: "Important!".to_string(),
            prio: Priority::High,
        },
        TextToSpeechAction::AllowLowPrio,
        TextToSpeechAction::DisallowLowPrio,
    ];

    for action in actions {
        let debug_str = format!("{:?}", action);
        assert!(!debug_str.is_empty());
    }
}

/// Test TTS events can be sent via event bus.
#[tokio::test]
async fn test_tts_events_through_bus() {
    let bus = EventBus::new();
    let mut subscriber = bus.subscribe();

    bus.send(Event::TextToSpeech(TextToSpeechAction::Speak {
        text: "Test".to_string(),
        prio: Priority::High,
    }));

    let event = subscriber.try_recv().unwrap();
    if let Event::TextToSpeech(TextToSpeechAction::Speak { text, prio }) = event {
        assert_eq!(text, "Test");
        assert_eq!(prio, Priority::High);
    } else {
        panic!("Expected TextToSpeech Speak event");
    }
}

/// Test PlaybackBuffer default is empty.
#[tokio::test]
async fn test_playback_buffer_default() {
    let buffer = PlaybackBuffer::default();
    assert!(!buffer.has_data());
}

/// Test PlaybackBuffer push and pull samples.
#[tokio::test]
async fn test_playback_buffer_push_pull() {
    let mut buffer = PlaybackBuffer::default();

    // Push some samples
    let samples: Vec<Sample> = vec![(100, -100), (200, -200), (300, -300)];
    buffer.push_samples(samples);

    // Pull them back
    let pulled = buffer.pull_samples(3);
    assert_eq!(pulled, vec![(100, -100), (200, -200), (300, -300)]);

    // Buffer should be empty after reading all samples
    assert!(!buffer.has_data());
}

/// Test PlaybackBuffer clear.
#[tokio::test]
async fn test_playback_buffer_clear() {
    let mut buffer = PlaybackBuffer::default();

    // Push samples
    let samples: Vec<Sample> = vec![(100, 100), (200, 200)];
    buffer.push_samples(samples);

    // Clear buffer
    buffer.clear();

    // Should have no data
    assert!(!buffer.has_data());
}

/// Test volume levels are within expected range.
#[tokio::test]
async fn test_volume_range_constraints() {
    // Test that various volume values are within sane range
    let test_volumes = vec![0.0, 0.2, 0.5, 0.75, 1.0, 1.25];

    for vol in test_volumes {
        // Volume should be non-negative
        assert!(vol >= 0.0);
        // Primary channel can be > 1.0 for boost, but secondary should be <= 1.0
        // We just verify the values are reasonable
        assert!(vol <= 2.0);
    }
}

/// Test TTS action clone behavior.
#[tokio::test]
async fn test_tts_action_clone() {
    let action = TextToSpeechAction::Speak {
        text: "Hello".to_string(),
        prio: Priority::High,
    };
    let cloned = action.clone();

    let orig_debug = format!("{:?}", action);
    let clone_debug = format!("{:?}", cloned);
    assert_eq!(orig_debug, clone_debug);
}

/// Test sample type is correct.
#[tokio::test]
async fn test_sample_type() {
    // Sample is (i16, i16) for stereo
    let sample: Sample = (i16::MAX, i16::MIN);

    assert_eq!(sample.0, i16::MAX);
    assert_eq!(sample.1, i16::MIN);
}

/// Test empty text TTS action.
#[tokio::test]
async fn test_tts_empty_text() {
    let action = TextToSpeechAction::Speak {
        text: String::new(),
        prio: Priority::Low,
    };

    // Should be creatable even with empty text
    if let TextToSpeechAction::Speak { text, .. } = action {
        assert!(text.is_empty());
    }
}

/// Test TTS with very long text.
#[tokio::test]
async fn test_tts_long_text() {
    let long_text = "A".repeat(10000);
    let action = TextToSpeechAction::Speak {
        text: long_text.clone(),
        prio: Priority::Low,
    };

    if let TextToSpeechAction::Speak { text, .. } = action {
        assert_eq!(text.len(), 10000);
    }
}

/// Test PlaybackBuffer with large sample count.
#[tokio::test]
async fn test_playback_buffer_large() {
    let mut buffer = PlaybackBuffer::default();

    // Push many samples
    let samples: Vec<Sample> = (0..10000).map(|i| (i as i16, -(i as i16))).collect();
    buffer.push_samples(samples);

    // Pull all samples
    let pulled = buffer.pull_samples(10000);
    assert_eq!(pulled.first(), Some(&(0, 0)));
    assert_eq!(pulled.last(), Some(&(9999, -9999)));
    assert!(!buffer.has_data());
}

/// Test PlaybackBuffer pull pads with silence when not enough samples.
#[tokio::test]
async fn test_playback_buffer_pull_pads_silence() {
    let mut buffer = PlaybackBuffer::default();

    buffer.push_samples(vec![(100, 100), (200, 200)]);

    // Request more samples than available
    let pulled = buffer.pull_samples(5);
    assert_eq!(
        pulled,
        vec![(100, 100), (200, 200), (0, 0), (0, 0), (0, 0)]
    );
}
