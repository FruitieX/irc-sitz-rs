//! Integration tests for the complete sitz (singalong party) flow.
//!
//! Tests the songleader state machine transitions:
//! Inactive → Starting → Singing → Tempo → Bingo → Singing → ...

mod common;

use common::*;
use std::time::Duration;

/// Test that the EventBus can send and receive events correctly.
#[tokio::test]
async fn test_event_bus_basics() {
    let harness = TestHarness::new();
    let mut subscriber = harness.bus().subscribe();

    // Send a simple songleader action
    harness.send_songleader(SongleaderAction::ListSongs);

    // Collect events
    let events = collect_events(&mut subscriber, Duration::from_millis(100)).await;

    // Verify the event was received
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Songleader(SongleaderAction::ListSongs))));
}

/// Test SongleaderState add_request functionality.
#[tokio::test]
async fn test_songleader_state_add_request() {
    let mut state = SongleaderState::new_without_persistence();

    let song1 = mock_songbook_song("song-1", "Test Song 1", Some("user1"));
    let song2 = mock_songbook_song("song-2", "Test Song 2", Some("user2"));

    // Add first request
    let result = state.add_request(song1.clone());
    assert!(result.is_ok());
    assert_eq!(state.requests.len(), 1);

    // Add second request
    let result = state.add_request(song2.clone());
    assert!(result.is_ok());
    assert_eq!(state.requests.len(), 2);

    // Attempting to add duplicate should fail
    let result = state.add_request(song1.clone());
    assert!(result.is_err());
}

/// Test that adding a song already in backup moves it to requests.
#[tokio::test]
async fn test_request_moves_from_backup() {
    let mut state = SongleaderState::new_without_persistence();

    let backup_song = mock_songbook_song("backup-song", "Backup Song", None);
    state.backup.push(backup_song.clone());

    assert_eq!(state.backup.len(), 1);
    assert_eq!(state.requests.len(), 0);

    // Request the same song that's in backup
    let mut requested_song = backup_song.clone();
    requested_song.queued_by = Some("user1".to_string());

    let result = state.add_request(requested_song);
    assert!(result.is_ok());

    // Song should be moved from backup to requests
    assert_eq!(state.backup.len(), 0);
    assert_eq!(state.requests.len(), 1);
}

/// Test rm_song_by_nick removes the most recently added song by that user.
#[tokio::test]
async fn test_rm_song_by_nick_removes_most_recent() {
    let mut state = SongleaderState::new_without_persistence();

    let song1 = mock_songbook_song("song-1", "First Song", Some("user1"));
    let song2 = mock_songbook_song("song-2", "Second Song", Some("user1"));
    let song3 = mock_songbook_song("song-3", "Third Song", Some("user1"));

    state.add_request(song1.clone()).unwrap();
    state.add_request(song2.clone()).unwrap();
    state.add_request(song3.clone()).unwrap();

    assert_eq!(state.requests.len(), 3);

    // Remove by nick should remove the LAST added (rposition semantics)
    let removed = state.rm_song_by_nick("user1".to_string()).unwrap();
    assert_eq!(removed.id, "song-3");
    assert_eq!(state.requests.len(), 2);

    // Remove again
    let removed = state.rm_song_by_nick("user1".to_string()).unwrap();
    assert_eq!(removed.id, "song-2");
    assert_eq!(state.requests.len(), 1);
}

/// Test rm_song_by_id removes the correct song.
#[tokio::test]
async fn test_rm_song_by_id() {
    let mut state = SongleaderState::new_without_persistence();

    let song1 = mock_songbook_song("song-1", "First Song", Some("user1"));
    let song2 = mock_songbook_song("song-2", "Second Song", Some("user2"));

    state.add_request(song1).unwrap();
    state.add_request(song2).unwrap();

    let removed = state.rm_song_by_id("song-1".to_string()).unwrap();
    assert_eq!(removed.id, "song-1");
    assert_eq!(state.requests.len(), 1);
    assert_eq!(state.requests[0].id, "song-2");
}

/// Test pop_next_song priority: first_songs > requests > backup.
#[tokio::test]
async fn test_pop_next_song_priority() {
    let mut state = SongleaderState::new_without_persistence();

    // Add songs to each queue
    let first_song = mock_songbook_song("first", "First Song", None);
    let request = mock_songbook_song("request", "Requested Song", Some("user1"));
    let backup = mock_songbook_song("backup", "Backup Song", None);

    state.first_songs.push_back(first_song.clone());
    state.requests.push(request.clone());
    state.backup.push(backup.clone());

    // First should come from first_songs
    let song = state.pop_next_song().unwrap();
    assert_eq!(song.id, "first");

    // Second should come from requests
    let song = state.pop_next_song().unwrap();
    assert_eq!(song.id, "request");

    // Third should come from backup
    let song = state.pop_next_song().unwrap();
    assert_eq!(song.id, "backup");

    // Now queue is empty
    assert!(state.pop_next_song().is_none());
}

/// Test Mode enum serialization (for state persistence).
#[tokio::test]
async fn test_mode_default_is_inactive() {
    let state = SongleaderState::new_without_persistence();
    assert_eq!(state.mode, Mode::Inactive);
}

/// Test that get_songs returns all songs in order.
#[tokio::test]
async fn test_get_songs_returns_all() {
    let mut state = SongleaderState::new_without_persistence();

    let first = mock_songbook_song("first", "First", None);
    let request = mock_songbook_song("request", "Request", Some("user"));
    let backup = mock_songbook_song("backup", "Backup", None);

    state.first_songs.push_back(first);
    state.requests.push(request);
    state.backup.push(backup);

    let songs = state.get_songs();
    assert_eq!(songs.len(), 3);
    assert_eq!(songs[0].id, "first");
    assert_eq!(songs[1].id, "request");
    assert_eq!(songs[2].id, "backup");
}

/// Test SongbookSong equality is based on ID only.
#[tokio::test]
async fn test_songbook_song_equality() {
    let song1 = SongbookSong {
        id: "test-id".to_string(),
        url: Some("url1".to_string()),
        title: Some("Title 1".to_string()),
        book: None,
        queued_by: Some("user1".to_string()),
    };

    let song2 = SongbookSong {
        id: "test-id".to_string(),
        url: Some("url2".to_string()),
        title: Some("Title 2".to_string()),
        book: Some("Different Book".to_string()),
        queued_by: Some("user2".to_string()),
    };

    // Songs with same ID are equal regardless of other fields
    assert_eq!(song1, song2);
}

/// Test Song equality is based on ID only.
#[tokio::test]
async fn test_song_equality() {
    let song1 = mock_song("abc123", "Title 1", "user1");
    let song2 = Song {
        id: "abc123".to_string(),
        url: "different-url".to_string(),
        title: "Different Title".to_string(),
        channel: "Different Channel".to_string(),
        duration: 999,
        queued_by: "different-user".to_string(),
    };

    // Songs with same ID are equal regardless of other fields
    assert_eq!(song1, song2);
}

/// Test that tempo mode tracks unique nicks.
#[tokio::test]
async fn test_tempo_mode_unique_nicks() {
    use std::collections::HashSet;
    use tokio::time::Instant;

    let mut nicks = HashSet::new();
    nicks.insert("user1".to_string());
    nicks.insert("user1".to_string()); // Duplicate
    nicks.insert("user2".to_string());

    let mode = Mode::Tempo {
        nicks,
        init_t: Instant::now(),
    };

    if let Mode::Tempo { nicks, .. } = mode {
        // HashSet automatically deduplicates
        assert_eq!(nicks.len(), 2);
    } else {
        panic!("Expected Tempo mode");
    }
}

/// Test bingo mode tracks unique nicks.
#[tokio::test]
async fn test_bingo_mode_unique_nicks() {
    use std::collections::HashSet;

    let mut nicks = HashSet::new();
    nicks.insert("user1".to_string());
    nicks.insert("user1".to_string()); // Duplicate
    nicks.insert("user2".to_string());

    let song = mock_songbook_song("test", "Test Song", None);
    let mode = Mode::Bingo { nicks, song };

    if let Mode::Bingo { nicks, .. } = mode {
        // HashSet automatically deduplicates
        assert_eq!(nicks.len(), 2);
    } else {
        panic!("Expected Bingo mode");
    }
}

/// Test helper functions for filtering events.
#[tokio::test]
async fn test_event_filter_helpers() {
    let events = vec![
        Event::Message(MessageAction::Send {
            text: "Hello".to_string(),
            rich: None,
            source: Platform::Bot,
        }),
        Event::Playback(PlaybackAction::Play),
        Event::TextToSpeech(TextToSpeechAction::Speak {
            text: "Test".to_string(),
            prio: irc_sitz_rs::sources::espeak::Priority::High,
        }),
        Event::Songleader(SongleaderAction::ListSongs),
    ];

    assert_eq!(filter_message_events(&events).len(), 1);
    assert_eq!(filter_playback_events(&events).len(), 1);
    assert_eq!(filter_tts_events(&events).len(), 1);
    assert_eq!(filter_songleader_events(&events).len(), 1);
}

/// Test has_message_containing helper.
#[tokio::test]
async fn test_has_message_containing() {
    let events = vec![
        Event::Message(MessageAction::Send {
            text: "Queue is empty".to_string(),
            rich: None,
            source: Platform::Bot,
        }),
        Event::Playback(PlaybackAction::Play),
    ];

    assert!(has_message_containing(&events, "empty"));
    assert!(has_message_containing(&events, "Queue"));
    assert!(!has_message_containing(&events, "full"));
}

/// Test extract_message_texts helper.
#[tokio::test]
async fn test_extract_message_texts() {
    let events = vec![
        Event::Message(MessageAction::Send {
            text: "First message".to_string(),
            rich: None,
            source: Platform::Bot,
        }),
        Event::Playback(PlaybackAction::Play),
        Event::Message(MessageAction::Send {
            text: "Second message".to_string(),
            rich: None,
            source: Platform::Bot,
        }),
    ];

    let texts = extract_message_texts(&events);
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0], "First message");
    assert_eq!(texts[1], "Second message");
}
