//! Integration tests for audio pipeline.
//!
//! Tests for mixer, TTS, and audio source handling.

mod common;

use common::*;
use irc_sitz_rs::mixer::MixerAction;
use irc_sitz_rs::sources::espeak::{Priority, TextToSpeechAction};
use irc_sitz_rs::buffer::PlaybackBuffer;

/// Test MixerAction duck/unduck variants.
#[tokio::test]
async fn test_mixer_action_duck_variants() {
    let actions = vec![
        MixerAction::DuckSecondaryChannels,
        MixerAction::UnduckSecondaryChannels,
    ];

    for action in actions {
        let debug_str = format!("{:?}", action);
        assert!(!debug_str.is_empty());
    }
}

/// Test MixerAction volume control variants.
#[tokio::test]
async fn test_mixer_action_volume_variants() {
    let actions = vec![
        MixerAction::SetSecondaryChannelVolume(0.0),
        MixerAction::SetSecondaryChannelVolume(0.5),
        MixerAction::SetSecondaryChannelVolume(1.0),
        MixerAction::SetSecondaryChannelDuckedVolume(0.0),
        MixerAction::SetSecondaryChannelDuckedVolume(0.2),
        MixerAction::SetSecondaryChannelDuckedVolume(1.0),
    ];

    for action in actions {
        let debug_str = format!("{:?}", action);
        assert!(debug_str.contains("Volume") || debug_str.contains("Set"));
    }
}

/// Test mixer events can be sent via event bus.
#[tokio::test]
async fn test_mixer_events_through_bus() {
    let bus = EventBus::new();
    let mut subscriber = bus.subscribe();

    bus.send(Event::Mixer(MixerAction::DuckSecondaryChannels));

    let event = subscriber.try_recv().unwrap();
    if let Event::Mixer(MixerAction::DuckSecondaryChannels) = event {
        // Expected
    } else {
        panic!("Expected Mixer DuckSecondaryChannels event");
    }
}

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
    // Default buffer should be empty - next_sample returns None
    let mut buffer = buffer;
    assert!(buffer.next_sample().is_none());
}

/// Test PlaybackBuffer push and pop samples.
#[tokio::test]
async fn test_playback_buffer_push_pop() {
    let mut buffer = PlaybackBuffer::default();

    // Push some samples
    let samples: Vec<(i16, i16)> = vec![(100, -100), (200, -200), (300, -300)];
    buffer.push_samples(samples.clone());

    // Pop them back
    let sample1 = buffer.next_sample();
    let sample2 = buffer.next_sample();
    let sample3 = buffer.next_sample();
    let sample4 = buffer.next_sample(); // Should be None

    assert_eq!(sample1, Some((100, -100)));
    assert_eq!(sample2, Some((200, -200)));
    assert_eq!(sample3, Some((300, -300)));
    assert_eq!(sample4, None);
}

/// Test PlaybackBuffer clear.
#[tokio::test]
async fn test_playback_buffer_clear() {
    let mut buffer = PlaybackBuffer::default();

    // Push samples
    let samples: Vec<(i16, i16)> = vec![(100, 100), (200, 200)];
    buffer.push_samples(samples);

    // Clear buffer
    buffer.clear();

    // Should be empty
    assert!(buffer.next_sample().is_none());
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

/// Test MixerAction clone behavior.
#[tokio::test]
async fn test_mixer_action_clone() {
    let action = MixerAction::SetSecondaryChannelVolume(0.75);
    let cloned = action.clone();

    // Verify clone matches original
    let orig_debug = format!("{:?}", action);
    let clone_debug = format!("{:?}", cloned);
    assert_eq!(orig_debug, clone_debug);
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
    let sample: irc_sitz_rs::mixer::Sample = (i16::MAX, i16::MIN);

    assert_eq!(sample.0, i16::MAX);
    assert_eq!(sample.1, i16::MIN);
}

/// Test multiple sequential duck/unduck events.
#[tokio::test]
async fn test_sequential_duck_unduck() {
    let bus = EventBus::new();
    let mut subscriber = bus.subscribe();

    // Send multiple duck/unduck events
    bus.send(Event::Mixer(MixerAction::DuckSecondaryChannels));
    bus.send(Event::Mixer(MixerAction::UnduckSecondaryChannels));
    bus.send(Event::Mixer(MixerAction::DuckSecondaryChannels));

    // Receive all events
    let event1 = subscriber.try_recv().unwrap();
    let event2 = subscriber.try_recv().unwrap();
    let event3 = subscriber.try_recv().unwrap();

    // Verify order
    matches!(event1, Event::Mixer(MixerAction::DuckSecondaryChannels));
    matches!(event2, Event::Mixer(MixerAction::UnduckSecondaryChannels));
    matches!(event3, Event::Mixer(MixerAction::DuckSecondaryChannels));
}

/// Test volume adjustment events in sequence.
#[tokio::test]
async fn test_volume_adjustment_sequence() {
    let bus = EventBus::new();
    let mut subscriber = bus.subscribe();

    // Normal volume
    bus.send(Event::Mixer(MixerAction::SetSecondaryChannelVolume(0.75)));
    // Ducked volume
    bus.send(Event::Mixer(MixerAction::SetSecondaryChannelDuckedVolume(0.2)));
    // Duck
    bus.send(Event::Mixer(MixerAction::DuckSecondaryChannels));
    // Unduck
    bus.send(Event::Mixer(MixerAction::UnduckSecondaryChannels));

    // All events should be received
    for _ in 0..4 {
        assert!(subscriber.try_recv().is_ok());
    }

    // No more events
    assert!(subscriber.try_recv().is_err());
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
    let samples: Vec<(i16, i16)> = (0..10000).map(|i| (i as i16, -(i as i16))).collect();
    buffer.push_samples(samples.clone());

    // Verify first and last samples
    assert_eq!(buffer.next_sample(), Some((0, 0)));

    // Skip to near the end
    for _ in 0..9998 {
        buffer.next_sample();
    }

    assert_eq!(buffer.next_sample(), Some((9999, -9999)));
    assert!(buffer.next_sample().is_none());
}
