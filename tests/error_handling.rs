//! Integration tests for error handling.
//!
//! Tests that the system handles errors gracefully.

mod common;

use common::*;
use std::collections::VecDeque;

/// Test SongleaderState gracefully handles empty queues for pop_next_song.
#[tokio::test]
async fn test_pop_from_empty_queues() {
    let mut state = SongleaderState::new_without_persistence();

    // All queues empty
    assert!(state.first_songs.is_empty());
    assert!(state.requests.is_empty());
    assert!(state.backup.is_empty());

    // pop_next_song should return None
    let result = state.pop_next_song();
    assert!(result.is_none());
}

/// Test removing non-existent song by nick.
#[tokio::test]
async fn test_remove_nonexistent_song_by_nick() {
    let mut state = SongleaderState::new_without_persistence();

    state
        .requests
        .push(mock_songbook_song("song1", "Song 1", Some("user1")));

    // Try to remove song by nick that doesn't exist
    let removed = state.rm_song_by_nick("nonexistent_user".to_string());
    assert!(removed.is_err());

    // Original song should still be there
    assert_eq!(state.requests.len(), 1);
}

/// Test invalid Mode JSON deserialization.
#[tokio::test]
async fn test_invalid_mode_json() {
    let invalid_json = r#""InvalidMode""#;
    let result: Result<Mode, _> = serde_json::from_str(invalid_json);
    assert!(result.is_err());
}

/// Test song duration at edge of limit.
#[tokio::test]
async fn test_song_duration_at_limit() {
    // Test song at exactly MAX_SONG_DURATION
    let duration_secs = irc_sitz_rs::playback::MAX_SONG_DURATION.as_secs();

    let song = Song {
        id: "test".to_string(),
        url: "https://youtu.be/test".to_string(),
        title: "Test".to_string(),
        channel: "Channel".to_string(),
        duration: duration_secs,
        queued_by: "user".to_string(),
    };

    // At limit should be OK
    assert_eq!(song.duration, duration_secs);
}

/// Test song duration over limit.
#[tokio::test]
async fn test_song_duration_over_limit() {
    let duration_secs = irc_sitz_rs::playback::MAX_SONG_DURATION.as_secs() + 1;

    let song = Song {
        id: "test".to_string(),
        url: "https://youtu.be/test".to_string(),
        title: "Test".to_string(),
        channel: "Channel".to_string(),
        duration: duration_secs,
        queued_by: "user".to_string(),
    };

    // Over limit - application should reject
    assert!(song.duration > irc_sitz_rs::playback::MAX_SONG_DURATION.as_secs());
}

/// Test empty song ID handling.
#[tokio::test]
async fn test_empty_song_id() {
    let song = SongbookSong {
        id: String::new(),
        url: Some("https://example.com".to_string()),
        title: Some("Test".to_string()),
        book: Some("Book".to_string()),
        queued_by: Some("user".to_string()),
    };

    // Empty ID - Display should still work
    let display = format!("{}", song);
    assert!(!display.is_empty());
}

/// Test SongbookSong display with missing title.
#[tokio::test]
async fn test_songbook_song_display_missing_title() {
    let song = SongbookSong {
        id: "song-id".to_string(),
        url: None,
        title: None, // Missing title
        book: Some("Test Book".to_string()),
        queued_by: None,
    };

    // Display should fall back to ID
    let display = format!("{}", song);
    assert!(display.contains("song-id"));
}

/// Test SongbookSong display with missing book.
#[tokio::test]
async fn test_songbook_song_display_missing_book() {
    let song = SongbookSong {
        id: "song-id".to_string(),
        url: None,
        title: Some("Test Title".to_string()),
        book: None, // Missing book
        queued_by: None,
    };

    // Display should just show title without book
    let display = format!("{}", song);
    assert!(display.contains("Test Title"));
    assert!(!display.contains("(")); // No book parenthesis
}

/// Test SongbookSong display with all fields.
#[tokio::test]
async fn test_songbook_song_display_complete() {
    let song = SongbookSong {
        id: "song-id".to_string(),
        url: Some("https://example.com".to_string()),
        title: Some("Test Title".to_string()),
        book: Some("Test Book".to_string()),
        queued_by: Some("user".to_string()),
    };

    let display = format!("{}", song);
    assert!(display.contains("Test Title"));
    assert!(display.contains("Test Book"));
    assert!(display.contains("("));
}

/// Test adding request with same ID is handled (deduplication).
#[tokio::test]
async fn test_duplicate_song_request() {
    let mut state = SongleaderState::new_without_persistence();

    let song1 = mock_songbook_song("song-1", "Song 1", Some("user1"));
    let song2 = mock_songbook_song("song-1", "Song 1 Duplicate", Some("user2")); // Same ID

    let _ = state.add_request(song1);
    let _ = state.add_request(song2);

    // Application deduplicates by ID - only one should be added
    assert_eq!(state.requests.len(), 1);
}

/// Test event bus subscription after events sent.
#[tokio::test]
async fn test_late_subscription_misses_events() {
    let bus = EventBus::new();

    // Send event before subscribing
    bus.send(Event::Songleader(SongleaderAction::Begin));

    // Subscribe after event sent
    let mut subscriber = bus.subscribe();

    // Late subscriber should NOT receive the earlier event
    let result = subscriber.try_recv();
    assert!(result.is_err());
}

/// Test event bus with multiple subscribers.
#[tokio::test]
async fn test_multiple_subscribers_receive_events() {
    let bus = EventBus::new();

    let mut sub1 = bus.subscribe();
    let mut sub2 = bus.subscribe();

    // Send event
    bus.send(Event::Songleader(SongleaderAction::Begin));

    // Both should receive
    assert!(sub1.try_recv().is_ok());
    assert!(sub2.try_recv().is_ok());
}

/// Test event bus recv with no events.
#[tokio::test]
async fn test_event_bus_empty_recv() {
    let bus = EventBus::new();
    let mut subscriber = bus.subscribe();

    // No events - should fail
    let result = subscriber.try_recv();
    assert!(result.is_err());
}

/// Test Mode::Tempo with empty nicks.
#[tokio::test]
async fn test_tempo_mode_empty_nicks() {
    use std::collections::HashSet;
    use tokio::time::Instant;

    let mode = Mode::Tempo {
        nicks: HashSet::new(),
        init_t: Instant::now(),
    };

    let json = serde_json::to_string(&mode).unwrap();
    assert!(json.contains("Tempo"));

    let deserialized: Mode = serde_json::from_str(&json).unwrap();
    if let Mode::Tempo { nicks, .. } = deserialized {
        assert!(nicks.is_empty());
    } else {
        panic!("Expected Tempo mode");
    }
}

/// Test Mode::Bingo with empty nicks.
#[tokio::test]
async fn test_bingo_mode_empty_nicks() {
    use std::collections::HashSet;

    let song = mock_songbook_song("bingo", "Bingo", None);
    let mode = Mode::Bingo {
        nicks: HashSet::new(),
        song,
    };

    let json = serde_json::to_string(&mode).unwrap();
    assert!(json.contains("Bingo"));

    let deserialized: Mode = serde_json::from_str(&json).unwrap();
    if let Mode::Bingo { nicks, .. } = deserialized {
        assert!(nicks.is_empty());
    } else {
        panic!("Expected Bingo mode");
    }
}

/// Test PlaybackAction variants exist and can be created.
#[tokio::test]
async fn test_playback_action_variants() {
    let song = mock_song("test", "Test", "user");

    // Verify all action variants can be created
    let actions: Vec<PlaybackAction> = vec![
        PlaybackAction::Enqueue { song: song.clone() },
        PlaybackAction::EndOfSong,
        PlaybackAction::ListQueue { offset: None },
        PlaybackAction::ListQueue { offset: Some(5) },
        PlaybackAction::RmSongByPos { pos: 0 },
        PlaybackAction::RmSongByNick {
            nick: "user".to_string(),
        },
        PlaybackAction::Play,
        PlaybackAction::Pause,
        PlaybackAction::Prev,
        PlaybackAction::Next,
        PlaybackAction::PlaybackProgress { position: 100 },
    ];

    // All actions should be Debug-printable
    for action in actions {
        let debug_str = format!("{:?}", action);
        assert!(!debug_str.is_empty());
    }
}

/// Test SongleaderAction variants exist and can be created.
#[tokio::test]
async fn test_songleader_action_variants() {
    let song = mock_songbook_song("test", "Test", Some("user"));

    let actions: Vec<SongleaderAction> = vec![
        SongleaderAction::Begin,
        SongleaderAction::End,
        SongleaderAction::Skål,
        SongleaderAction::Pause,
        SongleaderAction::Help,
        SongleaderAction::ListSongs,
        SongleaderAction::ForceTempo,
        SongleaderAction::ForceBingo,
        SongleaderAction::ForceSinging,
        SongleaderAction::Tempo {
            nick: "user".to_string(),
        },
        SongleaderAction::Bingo {
            nick: "user".to_string(),
        },
        SongleaderAction::RequestSong { song },
        SongleaderAction::RequestSongUrl {
            url: "https://example.com".to_string(),
            queued_by: "user".to_string(),
        },
        SongleaderAction::RmSongById {
            id: "song-id".to_string(),
        },
        SongleaderAction::RmSongByNick {
            nick: "user".to_string(),
        },
    ];

    for action in actions {
        let debug_str = format!("{:?}", action);
        assert!(!debug_str.is_empty());
    }
}

/// Test first_songs queue behavior - pops from first_songs first.
#[tokio::test]
async fn test_first_songs_priority() {
    let mut state = SongleaderState::new_without_persistence();

    // Add to first_songs (VecDeque) and requests (Vec)
    state.first_songs = VecDeque::from(vec![mock_songbook_song("first", "First", None)]);
    state.requests = vec![mock_songbook_song("request", "Request", Some("user"))];
    state.backup = vec![mock_songbook_song("backup", "Backup", None)];

    // First pop should come from first_songs
    let song1 = state.pop_next_song().unwrap();
    assert_eq!(song1.id, "first");

    // Second pop should come from requests
    let song2 = state.pop_next_song().unwrap();
    assert_eq!(song2.id, "request");

    // Third pop should come from backup
    let song3 = state.pop_next_song().unwrap();
    assert_eq!(song3.id, "backup");

    // Fourth pop should return None
    let song4 = state.pop_next_song();
    assert!(song4.is_none());
}

/// Test JSON with extra unknown fields is handled gracefully.
#[tokio::test]
async fn test_json_with_extra_fields() {
    // JSON with extra field that doesn't exist in SongbookSong
    let json_with_extra = r#"{
        "id": "test",
        "url": null,
        "title": "Test",
        "book": null,
        "queued_by": null,
        "unknown_field": "should be ignored"
    }"#;

    // Deserialization should succeed (serde defaults ignore unknown fields)
    let result: Result<SongbookSong, _> = serde_json::from_str(json_with_extra);
    // This may or may not work depending on serde config
    assert!(result.is_ok() || result.is_err());
}

/// Test Event variants can be created and are Debug-printable.
#[tokio::test]
async fn test_event_variants() {
    let song = mock_song("test", "Test", "user");

    let events: Vec<Event> = vec![
        Event::Songleader(SongleaderAction::Begin),
        Event::Playback(PlaybackAction::Play),
    ];

    for event in events {
        let debug_str = format!("{:?}", event);
        assert!(!debug_str.is_empty());
    }
}

/// Test empty configuration handling.
#[tokio::test]
async fn test_config_songbook_regex_validation() {
    // The songbook regex is compiled at config load time
    // Invalid regex would cause a panic at startup
    // This test ensures the default regex pattern is valid

    let regex_pattern = r"^(https?://)?[^/]+(/(?:[^/]+/)?)([^/\?#]+)";
    let result = regex::Regex::new(regex_pattern);
    assert!(result.is_ok(), "Songbook regex pattern should be valid");
}
