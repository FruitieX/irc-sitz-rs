//! Integration tests for IRC command parsing.
//!
//! Tests the command parsing logic that converts IRC messages to Events.
//! Since message_to_action requires an IRC Message struct, we test the
//! command action enums directly and their expected behaviors.

mod common;

use common::*;
use irc_sitz_rs::irc::IrcAction;

/// Test IrcAction::SendMsg can be created and sent through bus.
#[tokio::test]
async fn test_irc_action_send_msg() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    let msg = "Test message to IRC channel".to_string();
    harness
        .bus()
        .send(Event::Irc(IrcAction::SendMsg(msg.clone())));

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    let irc_event = events
        .iter()
        .find(|e| matches!(e, Event::Irc(IrcAction::SendMsg(m)) if m == &msg));

    assert!(irc_event.is_some());
}

/// Test that PlaybackAction::ListQueue is the expected result for !queue command.
#[tokio::test]
async fn test_expected_queue_action() {
    // The !queue command should produce a ListQueue action
    let action = PlaybackAction::ListQueue { offset: None };

    if let PlaybackAction::ListQueue { offset } = action {
        assert!(offset.is_none());
    } else {
        panic!("Expected ListQueue");
    }

    let action_with_offset = PlaybackAction::ListQueue { offset: Some(5) };
    if let PlaybackAction::ListQueue { offset } = action_with_offset {
        assert_eq!(offset, Some(5));
    } else {
        panic!("Expected ListQueue with offset");
    }
}

/// Test SongleaderAction::Tempo event structure.
#[tokio::test]
async fn test_tempo_action_structure() {
    let action = SongleaderAction::Tempo {
        nick: "testuser".to_string(),
    };

    if let SongleaderAction::Tempo { nick } = action {
        assert_eq!(nick, "testuser");
    } else {
        panic!("Expected Tempo action");
    }
}

/// Test SongleaderAction::Bingo event structure.
#[tokio::test]
async fn test_bingo_action_structure() {
    let action = SongleaderAction::Bingo {
        nick: "testuser".to_string(),
    };

    if let SongleaderAction::Bingo { nick } = action {
        assert_eq!(nick, "testuser");
    } else {
        panic!("Expected Bingo action");
    }
}

/// Test SongleaderAction::RequestSongUrl structure.
#[tokio::test]
async fn test_request_song_url_action() {
    let action = SongleaderAction::RequestSongUrl {
        url: "https://example-songbook.com/test-song".to_string(),
        queued_by: "testuser".to_string(),
    };

    if let SongleaderAction::RequestSongUrl { url, queued_by } = action {
        assert!(url.contains("example-songbook.com"));
        assert_eq!(queued_by, "testuser");
    } else {
        panic!("Expected RequestSongUrl action");
    }
}

/// Test SongleaderAction::RmSongByNick structure.
#[tokio::test]
async fn test_rm_song_by_nick_action() {
    let action = SongleaderAction::RmSongByNick {
        nick: "testuser".to_string(),
    };

    if let SongleaderAction::RmSongByNick { nick } = action {
        assert_eq!(nick, "testuser");
    } else {
        panic!("Expected RmSongByNick action");
    }
}

/// Test SongleaderAction::RmSongById structure.
#[tokio::test]
async fn test_rm_song_by_id_action() {
    let action = SongleaderAction::RmSongById {
        id: "test-song-id".to_string(),
    };

    if let SongleaderAction::RmSongById { id } = action {
        assert_eq!(id, "test-song-id");
    } else {
        panic!("Expected RmSongById action");
    }
}

/// Test admin command actions: ForceTempo, ForceBingo, ForceSinging.
#[tokio::test]
async fn test_admin_force_actions() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    // Send force tempo
    harness.send_songleader(SongleaderAction::ForceTempo);
    harness.send_songleader(SongleaderAction::ForceBingo);
    harness.send_songleader(SongleaderAction::ForceSinging);

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::ForceTempo))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::ForceBingo))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::ForceSinging))));
}

/// Test lifecycle actions: Begin, End, Pause.
#[tokio::test]
async fn test_lifecycle_actions() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    harness.send_songleader(SongleaderAction::Begin);
    harness.send_songleader(SongleaderAction::Pause);
    harness.send_songleader(SongleaderAction::End);

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::Begin))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::Pause))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::End))));
}

/// Test informational actions: ListSongs, Help.
#[tokio::test]
async fn test_info_actions() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    harness.send_songleader(SongleaderAction::ListSongs);
    harness.send_songleader(SongleaderAction::Help);

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::ListSongs))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::Help))));
}

/// Test PlaybackAction::RmSongByNick structure.
#[tokio::test]
async fn test_playback_rm_by_nick_action() {
    let action = PlaybackAction::RmSongByNick {
        nick: "remover".to_string(),
    };

    if let PlaybackAction::RmSongByNick { nick } = action {
        assert_eq!(nick, "remover");
    } else {
        panic!("Expected RmSongByNick");
    }
}

/// Test PlaybackAction::RmSongByPos structure.
#[tokio::test]
async fn test_playback_rm_by_pos_action() {
    let action = PlaybackAction::RmSongByPos { pos: 3 };

    if let PlaybackAction::RmSongByPos { pos } = action {
        assert_eq!(pos, 3);
    } else {
        panic!("Expected RmSongByPos");
    }
}

/// Test music control actions: Play, Pause, Next, Prev.
#[tokio::test]
async fn test_music_control_actions() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    harness.send_playback(PlaybackAction::Play);
    harness.send_playback(PlaybackAction::Pause);
    harness.send_playback(PlaybackAction::Next);
    harness.send_playback(PlaybackAction::Prev);

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Playback(PlaybackAction::Play))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Playback(PlaybackAction::Pause))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Playback(PlaybackAction::Next))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Playback(PlaybackAction::Prev))));
}

/// Test TextToSpeechAction::Speak with different priorities.
#[tokio::test]
async fn test_tts_speak_action() {
    use irc_sitz_rs::sources::espeak::{Priority, TextToSpeechAction};

    let low_prio = TextToSpeechAction::Speak {
        text: "Low priority message".to_string(),
        prio: Priority::Low,
    };

    let high_prio = TextToSpeechAction::Speak {
        text: "High priority message".to_string(),
        prio: Priority::High,
    };

    if let TextToSpeechAction::Speak { text, prio } = low_prio {
        assert_eq!(prio, Priority::Low);
        assert!(text.contains("Low"));
    } else {
        panic!("Expected Speak action");
    }

    if let TextToSpeechAction::Speak { text, prio } = high_prio {
        assert_eq!(prio, Priority::High);
        assert!(text.contains("High"));
    } else {
        panic!("Expected Speak action");
    }
}

/// Test TTS allow/disallow low priority actions.
#[tokio::test]
async fn test_tts_priority_control() {
    use irc_sitz_rs::sources::espeak::TextToSpeechAction;

    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    harness
        .bus()
        .send(Event::TextToSpeech(TextToSpeechAction::AllowLowPrio));
    harness
        .bus()
        .send(Event::TextToSpeech(TextToSpeechAction::DisallowLowPrio));

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    assert!(events
        .iter()
        .any(|e| matches!(e, Event::TextToSpeech(TextToSpeechAction::AllowLowPrio))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::TextToSpeech(TextToSpeechAction::DisallowLowPrio))));
}

/// Test MixerAction variants.
#[tokio::test]
async fn test_mixer_actions() {
    use irc_sitz_rs::mixer::MixerAction;

    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    harness
        .bus()
        .send(Event::Mixer(MixerAction::DuckSecondaryChannels));
    harness
        .bus()
        .send(Event::Mixer(MixerAction::UnduckSecondaryChannels));
    harness
        .bus()
        .send(Event::Mixer(MixerAction::SetSecondaryChannelVolume(0.5)));
    harness
        .bus()
        .send(Event::Mixer(MixerAction::SetSecondaryChannelDuckedVolume(
            0.3,
        )));

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Mixer(MixerAction::DuckSecondaryChannels))));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Mixer(MixerAction::UnduckSecondaryChannels))));
    assert!(events.iter().any(
        |e| matches!(e, Event::Mixer(MixerAction::SetSecondaryChannelVolume(v)) if (*v - 0.5).abs() < 0.001)
    ));
    assert!(events.iter().any(
        |e| matches!(e, Event::Mixer(MixerAction::SetSecondaryChannelDuckedVolume(v)) if (*v - 0.3).abs() < 0.001)
    ));
}

/// Test SongleaderAction::RequestSong with a direct SongbookSong.
#[tokio::test]
async fn test_request_song_direct() {
    let song = mock_songbook_song("force-test", "Force Test Song", Some("admin"));
    let action = SongleaderAction::RequestSong { song: song.clone() };

    if let SongleaderAction::RequestSong { song: s } = action {
        assert_eq!(s.id, "force-test");
        assert_eq!(s.title, Some("Force Test Song".to_string()));
        assert_eq!(s.queued_by, Some("admin".to_string()));
    } else {
        panic!("Expected RequestSong action");
    }
}

/// Test SongleaderAction::Skål.
#[tokio::test]
async fn test_skål_action() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    harness.send_songleader(SongleaderAction::Skål);

    let events = collect_events(&mut subscriber, std::time::Duration::from_millis(100)).await;

    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::Skål))));
}
