//! Integration tests for music queue management (playback module).
//!
//! Tests the playback queue operations: enqueue, skip, remove, list, etc.

mod common;

use common::*;
use std::time::Duration;

/// Test Song struct equality is based on ID.
#[tokio::test]
async fn test_song_equality_by_id() {
    let song1 = mock_song("abc123", "Original Title", "user1");
    let song2 = mock_song("abc123", "Different Title", "user2");
    let song3 = mock_song("xyz789", "Original Title", "user1");

    // Same ID = equal
    assert_eq!(song1, song2);
    // Different ID = not equal
    assert_ne!(song1, song3);
}

/// Test Song struct serialization/deserialization.
#[tokio::test]
async fn test_song_serialization() {
    let song = mock_song("test-id", "Test Title", "test-user");

    let json = serde_json::to_string(&song).unwrap();
    let deserialized: Song = serde_json::from_str(&json).unwrap();

    assert_eq!(song.id, deserialized.id);
    assert_eq!(song.title, deserialized.title);
    assert_eq!(song.url, deserialized.url);
    assert_eq!(song.channel, deserialized.channel);
    assert_eq!(song.duration, deserialized.duration);
    assert_eq!(song.queued_by, deserialized.queued_by);
}

/// Test MAX_SONG_DURATION constant is 10 minutes.
#[tokio::test]
async fn test_max_song_duration() {
    use irc_sitz_rs::playback::MAX_SONG_DURATION;

    assert_eq!(MAX_SONG_DURATION, Duration::from_secs(10 * 60));
    assert_eq!(MAX_SONG_DURATION.as_secs(), 600);
}

/// Test PlaybackAction enum variants can be cloned and debugged.
#[tokio::test]
async fn test_playback_action_clone_and_debug() {
    let actions = vec![
        PlaybackAction::Enqueue {
            song: mock_song("test", "Test", "user"),
        },
        PlaybackAction::EndOfSong,
        PlaybackAction::ListQueue { offset: None },
        PlaybackAction::ListQueue { offset: Some(5) },
        PlaybackAction::RmSongByPos { pos: 3 },
        PlaybackAction::RmSongByNick {
            nick: "testuser".to_string(),
        },
        PlaybackAction::Play,
        PlaybackAction::Pause,
        PlaybackAction::Prev,
        PlaybackAction::Next,
        PlaybackAction::PlaybackProgress { position: 120 },
    ];

    // All actions should be cloneable and debuggable
    for action in actions {
        let cloned = action.clone();
        let debug_str = format!("{:?}", cloned);
        assert!(!debug_str.is_empty());
    }
}

/// Test that Events containing PlaybackAction can be sent through EventBus.
#[tokio::test]
async fn test_playback_events_through_bus() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    let song = mock_song("test-song", "Test Song", "testuser");

    // Send various playback events
    harness
        .bus()
        .send(Event::Playback(PlaybackAction::Enqueue { song }));
    harness.bus().send(Event::Playback(PlaybackAction::Play));
    harness
        .bus()
        .send(Event::Playback(PlaybackAction::ListQueue { offset: None }));
    harness.bus().send(Event::Playback(PlaybackAction::Pause));
    harness.bus().send(Event::Playback(PlaybackAction::Next));

    let events = collect_events(&mut subscriber, Duration::from_millis(100)).await;
    let playback_events = filter_playback_events(&events);

    assert_eq!(playback_events.len(), 5);
}

/// Test PlaybackProgress event contains position.
#[tokio::test]
async fn test_playback_progress_event() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    harness
        .bus()
        .send(Event::Playback(PlaybackAction::PlaybackProgress {
            position: 42,
        }));

    let events = collect_events(&mut subscriber, Duration::from_millis(100)).await;

    let progress_event = events.iter().find(|e| {
        matches!(
            e,
            Event::Playback(PlaybackAction::PlaybackProgress { position: 42 })
        )
    });

    assert!(progress_event.is_some());
}

/// Test RmSongByNick matches correct user.
#[tokio::test]
async fn test_rm_song_by_nick_action() {
    let action = PlaybackAction::RmSongByNick {
        nick: "testuser".to_string(),
    };

    if let PlaybackAction::RmSongByNick { nick } = action {
        assert_eq!(nick, "testuser");
    } else {
        panic!("Expected RmSongByNick");
    }
}

/// Test RmSongByPos contains correct position.
#[tokio::test]
async fn test_rm_song_by_pos_action() {
    let action = PlaybackAction::RmSongByPos { pos: 7 };

    if let PlaybackAction::RmSongByPos { pos } = action {
        assert_eq!(pos, 7);
    } else {
        panic!("Expected RmSongByPos");
    }
}

/// Test ListQueue with and without offset.
#[tokio::test]
async fn test_list_queue_action_variants() {
    let action1 = PlaybackAction::ListQueue { offset: None };
    let action2 = PlaybackAction::ListQueue { offset: Some(10) };

    if let PlaybackAction::ListQueue { offset } = action1 {
        assert!(offset.is_none());
    } else {
        panic!("Expected ListQueue");
    }

    if let PlaybackAction::ListQueue { offset } = action2 {
        assert_eq!(offset, Some(10));
    } else {
        panic!("Expected ListQueue");
    }
}

/// Test song with valid duration passes duration check.
#[tokio::test]
async fn test_song_duration_within_limit() {
    use irc_sitz_rs::playback::MAX_SONG_DURATION;

    let short_song = mock_song_with_duration("short", "Short Song", "user", 180); // 3 min
    let exact_song = mock_song_with_duration("exact", "Exact Song", "user", 600); // 10 min exactly

    assert!(short_song.duration <= MAX_SONG_DURATION.as_secs());
    assert!(exact_song.duration <= MAX_SONG_DURATION.as_secs());
}

/// Test song exceeding duration limit.
#[tokio::test]
async fn test_song_duration_exceeds_limit() {
    use irc_sitz_rs::playback::MAX_SONG_DURATION;

    let long_song = mock_song_with_duration("long", "Long Song", "user", 601); // 10 min + 1 sec
    let very_long = mock_song_with_duration("very", "Very Long", "user", 3600); // 1 hour

    assert!(long_song.duration > MAX_SONG_DURATION.as_secs());
    assert!(very_long.duration > MAX_SONG_DURATION.as_secs());
}

/// Test multiple songs can be created with unique IDs.
#[tokio::test]
async fn test_multiple_unique_songs() {
    let songs: Vec<Song> = (0..10)
        .map(|i| mock_song(&format!("id-{i}"), &format!("Song {i}"), "user"))
        .collect();

    // All songs should have unique IDs
    for i in 0..songs.len() {
        for j in (i + 1)..songs.len() {
            assert_ne!(songs[i], songs[j]);
        }
    }
}

/// Test song URL format.
#[tokio::test]
async fn test_song_url_format() {
    let song = mock_song("abc123xyz", "Test", "user");

    assert!(song.url.starts_with("https://youtu.be/"));
    assert!(song.url.contains("abc123xyz"));
}

/// Test Enqueue action contains complete song data.
#[tokio::test]
async fn test_enqueue_action_song_data() {
    let song = mock_song("test-id", "Test Title", "test-user");
    let action = PlaybackAction::Enqueue { song: song.clone() };

    if let PlaybackAction::Enqueue { song: enqueued } = action {
        assert_eq!(enqueued.id, "test-id");
        assert_eq!(enqueued.title, "Test Title");
        assert_eq!(enqueued.queued_by, "test-user");
    } else {
        panic!("Expected Enqueue");
    }
}
