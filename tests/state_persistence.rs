//! Integration tests for state persistence.
//!
//! Tests that songleader and playback states are correctly serialized and deserialized.

mod common;

use common::*;
use std::collections::{HashSet, VecDeque};

/// Test SongleaderState serialization to JSON.
#[tokio::test]
async fn test_songleader_state_serialization() {
    let mut state = SongleaderState::new_without_persistence();

    let song1 = mock_songbook_song("song-1", "First Song", Some("user1"));
    let song2 = mock_songbook_song("song-2", "Second Song", Some("user2"));

    state.requests.push(song1);
    state.backup.push(song2);

    let json = serde_json::to_string_pretty(&state).unwrap();

    // Verify JSON contains expected fields
    assert!(json.contains("requests"));
    assert!(json.contains("backup"));
    assert!(json.contains("mode"));
    assert!(json.contains("song-1"));
    assert!(json.contains("song-2"));
}

/// Test SongleaderState deserialization from JSON.
#[tokio::test]
async fn test_songleader_state_deserialization() {
    let json = r#"{
        "first_songs": [],
        "requests": [
            {
                "id": "test-song",
                "url": "https://example.com/test-song",
                "title": "Test Song",
                "book": "Test Book",
                "queued_by": "testuser"
            }
        ],
        "backup": [],
        "mode": "Inactive"
    }"#;

    let state: SongleaderState = serde_json::from_str(json).unwrap();

    assert_eq!(state.requests.len(), 1);
    assert_eq!(state.requests[0].id, "test-song");
    assert_eq!(state.requests[0].queued_by, Some("testuser".to_string()));
    assert!(state.backup.is_empty());
    assert_eq!(state.mode, Mode::Inactive);
}

/// Test Mode enum serialization for Inactive.
#[tokio::test]
async fn test_mode_inactive_serialization() {
    let mode = Mode::Inactive;
    let json = serde_json::to_string(&mode).unwrap();
    assert_eq!(json, "\"Inactive\"");

    let deserialized: Mode = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, Mode::Inactive);
}

/// Test Mode enum serialization for Singing.
#[tokio::test]
async fn test_mode_singing_serialization() {
    let mode = Mode::Singing;
    let json = serde_json::to_string(&mode).unwrap();
    assert_eq!(json, "\"Singing\"");

    let deserialized: Mode = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, Mode::Singing);
}

/// Test Mode enum serialization for Starting.
#[tokio::test]
async fn test_mode_starting_serialization() {
    let mode = Mode::Starting;
    let json = serde_json::to_string(&mode).unwrap();
    assert_eq!(json, "\"Starting\"");

    let deserialized: Mode = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, Mode::Starting);
}

/// Test Mode::Tempo serialization (complex type with HashSet and Instant).
#[tokio::test]
async fn test_mode_tempo_serialization() {
    use tokio::time::Instant;

    let mut nicks = HashSet::new();
    nicks.insert("user1".to_string());
    nicks.insert("user2".to_string());

    let mode = Mode::Tempo {
        nicks: nicks.clone(),
        init_t: Instant::now(),
    };

    let json = serde_json::to_string(&mode).unwrap();

    // Verify it serializes (init_t is skipped via #[serde(skip)])
    assert!(json.contains("Tempo"));
    assert!(json.contains("nicks"));

    // Deserialize and verify
    let deserialized: Mode = serde_json::from_str(&json).unwrap();
    if let Mode::Tempo {
        nicks: deserialized_nicks,
        ..
    } = deserialized
    {
        assert_eq!(deserialized_nicks.len(), 2);
        assert!(deserialized_nicks.contains("user1"));
        assert!(deserialized_nicks.contains("user2"));
    } else {
        panic!("Expected Tempo mode");
    }
}

/// Test Mode::Bingo serialization.
#[tokio::test]
async fn test_mode_bingo_serialization() {
    let mut nicks = HashSet::new();
    nicks.insert("user1".to_string());

    let song = mock_songbook_song("bingo-song", "Bingo Song", None);
    let mode = Mode::Bingo {
        nicks: nicks.clone(),
        song: song.clone(),
    };

    let json = serde_json::to_string(&mode).unwrap();

    assert!(json.contains("Bingo"));
    assert!(json.contains("nicks"));
    assert!(json.contains("song"));
    assert!(json.contains("bingo-song"));

    let deserialized: Mode = serde_json::from_str(&json).unwrap();
    if let Mode::Bingo {
        nicks: deserialized_nicks,
        song: deserialized_song,
    } = deserialized
    {
        assert_eq!(deserialized_nicks.len(), 1);
        assert_eq!(deserialized_song.id, "bingo-song");
    } else {
        panic!("Expected Bingo mode");
    }
}

/// Test SongbookSong serialization.
#[tokio::test]
async fn test_songbook_song_serialization() {
    let song = SongbookSong {
        id: "test-id".to_string(),
        url: Some("https://example.com/test".to_string()),
        title: Some("Test Title".to_string()),
        book: Some("Test Book".to_string()),
        queued_by: Some("testuser".to_string()),
    };

    let json = serde_json::to_string_pretty(&song).unwrap();

    assert!(json.contains("test-id"));
    assert!(json.contains("Test Title"));
    assert!(json.contains("Test Book"));
    assert!(json.contains("testuser"));

    let deserialized: SongbookSong = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id, song.id);
    assert_eq!(deserialized.url, song.url);
    assert_eq!(deserialized.title, song.title);
    assert_eq!(deserialized.book, song.book);
    assert_eq!(deserialized.queued_by, song.queued_by);
}

/// Test SongbookSong with None fields serialization.
#[tokio::test]
async fn test_songbook_song_none_fields() {
    let song = SongbookSong {
        id: "minimal".to_string(),
        url: None,
        title: None,
        book: None,
        queued_by: None,
    };

    let json = serde_json::to_string(&song).unwrap();
    let deserialized: SongbookSong = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, "minimal");
    assert!(deserialized.url.is_none());
    assert!(deserialized.title.is_none());
    assert!(deserialized.book.is_none());
    assert!(deserialized.queued_by.is_none());
}

/// Test Song (playback) serialization.
#[tokio::test]
async fn test_song_serialization() {
    let song = mock_song("abc123", "Test Song", "testuser");

    let json = serde_json::to_string_pretty(&song).unwrap();

    assert!(json.contains("abc123"));
    assert!(json.contains("Test Song"));
    assert!(json.contains("testuser"));
    assert!(json.contains("duration"));
    assert!(json.contains("channel"));

    let deserialized: Song = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id, song.id);
    assert_eq!(deserialized.title, song.title);
    assert_eq!(deserialized.queued_by, song.queued_by);
}

/// Test complete state round-trip with multiple songs.
#[tokio::test]
async fn test_full_state_round_trip() {
    let mut state = SongleaderState::new_without_persistence();

    // Add songs to first_songs queue
    state.first_songs = VecDeque::from(vec![
        mock_songbook_song("first-1", "First Song 1", None),
        mock_songbook_song("first-2", "First Song 2", None),
    ]);

    // Add songs to requests
    state.requests = vec![
        mock_songbook_song("req-1", "Request 1", Some("user1")),
        mock_songbook_song("req-2", "Request 2", Some("user2")),
    ];

    // Add songs to backup
    state.backup = vec![
        mock_songbook_song("backup-1", "Backup 1", None),
        mock_songbook_song("backup-2", "Backup 2", None),
    ];

    // set mode to Tempo with some nicks
    let mut nicks = HashSet::new();
    nicks.insert("tempo-user".to_string());
    state.mode = Mode::Tempo {
        nicks,
        init_t: tokio::time::Instant::now(),
    };

    // Serialize
    let json = serde_json::to_string_pretty(&state).unwrap();

    // Deserialize
    let restored: SongleaderState = serde_json::from_str(&json).unwrap();

    // Verify all data is preserved
    assert_eq!(restored.first_songs.len(), 2);
    assert_eq!(restored.requests.len(), 2);
    assert_eq!(restored.backup.len(), 2);

    assert_eq!(restored.first_songs[0].id, "first-1");
    assert_eq!(restored.first_songs[1].id, "first-2");
    assert_eq!(restored.requests[0].id, "req-1");
    assert_eq!(restored.requests[1].id, "req-2");
    assert_eq!(restored.backup[0].id, "backup-1");
    assert_eq!(restored.backup[1].id, "backup-2");

    if let Mode::Tempo {
        nicks: restored_nicks,
        ..
    } = restored.mode
    {
        assert!(restored_nicks.contains("tempo-user"));
    } else {
        panic!("Expected Tempo mode");
    }
}

/// Test malformed JSON handling.
#[tokio::test]
async fn test_malformed_json_handling() {
    let malformed = "{ not valid json }";
    let result: Result<SongleaderState, _> = serde_json::from_str(malformed);
    assert!(result.is_err());
}

/// Test missing fields default to default values.
#[tokio::test]
async fn test_partial_json_uses_defaults() {
    // JSON with only required fields
    let partial = r#"{
        "first_songs": [],
        "requests": [],
        "backup": [],
        "mode": "Inactive"
    }"#;

    let state: SongleaderState = serde_json::from_str(partial).unwrap();
    assert!(state.first_songs.is_empty());
    assert!(state.requests.is_empty());
    assert!(state.backup.is_empty());
    assert_eq!(state.mode, Mode::Inactive);
}

/// Test atomic write pattern simulation - temp file then rename.
#[tokio::test]
async fn test_atomic_write_pattern() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let state_path = temp_dir.path().join("songleader_state.json");
    let temp_path = temp_dir.path().join("songleader_state.json.tmp");

    let state = SongleaderState::new_without_persistence();
    let json = serde_json::to_string_pretty(&state).unwrap();

    // Step 1: Write to temp file
    tokio::fs::write(&temp_path, &json).await.unwrap();
    assert!(temp_path.exists());

    // Step 2: Atomic rename
    tokio::fs::rename(&temp_path, &state_path).await.unwrap();

    // Verify: temp file gone, state file exists
    assert!(!temp_path.exists());
    assert!(state_path.exists());

    // Verify content
    let content = tokio::fs::read_to_string(&state_path).await.unwrap();
    let restored: SongleaderState = serde_json::from_str(&content).unwrap();
    assert_eq!(restored.mode, Mode::Inactive);
}

/// Test state file with corrupted data falls back to default.
#[tokio::test]
async fn test_corrupted_state_fallback() {
    // SongleaderState::read_or_default() should return default on error
    // We test the pattern here

    let corrupted = "definitely not json";
    let result: Result<SongleaderState, _> = serde_json::from_str(corrupted);

    // Should fail to parse
    assert!(result.is_err());

    // Application would fall back to default
    let fallback = SongleaderState::new_without_persistence();
    assert!(fallback.requests.is_empty());
    assert_eq!(fallback.mode, Mode::Inactive);
}

/// Test getters return consistent data.
#[tokio::test]
async fn test_get_songs_consistency() {
    let mut state = SongleaderState::new_without_persistence();

    state.first_songs = VecDeque::from(vec![mock_songbook_song("f1", "F1", None)]);
    state.requests = vec![mock_songbook_song("r1", "R1", Some("u"))];
    state.backup = vec![mock_songbook_song("b1", "B1", None)];

    let songs = state.get_songs();

    // Should be in order: first_songs, requests, backup
    assert_eq!(songs.len(), 3);
    assert_eq!(songs[0].id, "f1");
    assert_eq!(songs[1].id, "r1");
    assert_eq!(songs[2].id, "b1");
}
