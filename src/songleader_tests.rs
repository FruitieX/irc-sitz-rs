//! Unit tests for the songleader module

#[cfg(test)]
mod tests {
    use crate::songbook::SongbookSong;
    use crate::songleader::{Mode, SongleaderAction, SongleaderState};
    use std::collections::HashSet;

    fn make_test_song(id: &str) -> SongbookSong {
        SongbookSong {
            id: id.to_string(),
            url: Some(format!("https://example.com/{}", id)),
            title: Some(format!("Test Song {}", id)),
            book: Some("Test Book".to_string()),
            queued_by: Some("testuser".to_string()),
        }
    }

    #[test]
    fn test_mode_default_is_inactive() {
        let mode = Mode::default();
        assert_eq!(mode, Mode::Inactive);
    }

    #[test]
    fn test_mode_equality() {
        assert_eq!(Mode::Inactive, Mode::Inactive);
        assert_eq!(Mode::Starting, Mode::Starting);
        assert_eq!(Mode::Singing, Mode::Singing);
        assert_ne!(Mode::Inactive, Mode::Singing);
    }

    #[test]
    fn test_mode_tempo_with_nicks() {
        use tokio::time::Instant;

        let nicks: HashSet<String> = vec!["user1".to_string(), "user2".to_string()]
            .into_iter()
            .collect();

        let mode = Mode::Tempo {
            nicks: nicks.clone(),
            init_t: Instant::now(),
        };

        if let Mode::Tempo {
            nicks: mode_nicks, ..
        } = mode
        {
            assert_eq!(mode_nicks.len(), 2);
            assert!(mode_nicks.contains("user1"));
            assert!(mode_nicks.contains("user2"));
        } else {
            panic!("Expected Mode::Tempo");
        }
    }

    #[test]
    fn test_mode_bingo_with_song() {
        let song = make_test_song("bingo-song");
        let nicks: HashSet<String> = HashSet::new();

        let mode = Mode::Bingo {
            nicks,
            song: song.clone(),
        };

        if let Mode::Bingo {
            song: mode_song, ..
        } = mode
        {
            assert_eq!(mode_song.id, "bingo-song");
        } else {
            panic!("Expected Mode::Bingo");
        }
    }

    #[test]
    fn test_songleader_state_default() {
        let state = SongleaderState::default();

        assert!(state.first_songs.is_empty());
        assert!(state.requests.is_empty());
        assert!(state.backup.is_empty());
        assert_eq!(state.mode, Mode::Inactive);
    }

    #[test]
    fn test_songleader_state_get_songs_empty() {
        let state = SongleaderState::default();
        let songs = state.get_songs();
        assert!(songs.is_empty());
    }

    #[test]
    fn test_songleader_state_get_songs_combined() {
        let mut state = SongleaderState::default();

        state.first_songs.push_back(make_test_song("first-1"));
        state.requests.push(make_test_song("request-1"));
        state.requests.push(make_test_song("request-2"));
        state.backup.push(make_test_song("backup-1"));

        let songs = state.get_songs();

        assert_eq!(songs.len(), 4);
        // First songs come first
        assert_eq!(songs[0].id, "first-1");
        // Then requests
        assert_eq!(songs[1].id, "request-1");
        assert_eq!(songs[2].id, "request-2");
        // Then backup
        assert_eq!(songs[3].id, "backup-1");
    }

    #[test]
    fn test_songleader_state_pop_next_song_priority() {
        let mut state = SongleaderState::default();

        state.first_songs.push_back(make_test_song("first-1"));
        state.requests.push(make_test_song("request-1"));
        state.backup.push(make_test_song("backup-1"));

        // First songs have highest priority
        let song = state.pop_next_song();
        assert!(song.is_some());
        assert_eq!(song.unwrap().id, "first-1");

        // Requests come next (since first_songs is now empty)
        let song = state.pop_next_song();
        assert!(song.is_some());
        assert_eq!(song.unwrap().id, "request-1");

        // Backup comes last
        let song = state.pop_next_song();
        assert!(song.is_some());
        assert_eq!(song.unwrap().id, "backup-1");

        // Now empty
        let song = state.pop_next_song();
        assert!(song.is_none());
    }

    #[tokio::test]
    async fn test_songleader_state_add_request_success() {
        let mut state = SongleaderState::default();

        let song = make_test_song("new-request");
        let result = state.add_request(song.clone());

        assert!(result.is_ok());
        assert_eq!(state.requests.len(), 1);
        assert_eq!(state.requests[0].id, "new-request");
    }

    #[tokio::test]
    async fn test_songleader_state_add_request_duplicate_fails() {
        let mut state = SongleaderState::default();

        let song = make_test_song("duplicate");
        state.add_request(song.clone()).unwrap();

        // Adding the same song again should fail
        let result = state.add_request(song);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("already requested"));
    }

    #[tokio::test]
    async fn test_songleader_state_add_request_moves_from_backup() {
        let mut state = SongleaderState::default();

        let song = make_test_song("in-backup");
        state.backup.push(song.clone());

        // Adding a song that's in backup should move it to requests
        let result = state.add_request(song);
        assert!(result.is_ok());

        assert!(state.backup.is_empty());
        assert_eq!(state.requests.len(), 1);
        assert_eq!(state.requests[0].id, "in-backup");
    }

    #[tokio::test]
    async fn test_songleader_state_rm_song_by_id() {
        let mut state = SongleaderState::default();

        state.requests.push(make_test_song("song-1"));
        state.requests.push(make_test_song("song-2"));
        state.requests.push(make_test_song("song-3"));

        let result = state.rm_song_by_id("song-2".to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "song-2");

        assert_eq!(state.requests.len(), 2);
        assert_eq!(state.requests[0].id, "song-1");
        assert_eq!(state.requests[1].id, "song-3");
    }

    #[test]
    fn test_songleader_state_rm_song_by_id_not_found() {
        let mut state = SongleaderState::default();

        let result = state.rm_song_by_id("nonexistent".to_string());
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_songleader_state_rm_song_by_nick() {
        let mut state = SongleaderState::default();

        let mut song1 = make_test_song("song-1");
        song1.queued_by = Some("alice".to_string());

        let mut song2 = make_test_song("song-2");
        song2.queued_by = Some("bob".to_string());

        let mut song3 = make_test_song("song-3");
        song3.queued_by = Some("alice".to_string()); // Alice's second song

        state.requests.push(song1);
        state.requests.push(song2);
        state.requests.push(song3);

        // Should remove alice's LAST song (song-3)
        let result = state.rm_song_by_nick("alice".to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "song-3");

        // song-1 (alice's first) should still be there
        assert_eq!(state.requests.len(), 2);
        assert!(state.requests.iter().any(|s| s.id == "song-1"));
    }

    #[test]
    fn test_songleader_state_rm_song_by_nick_not_found() {
        let mut state = SongleaderState::default();

        let result = state.rm_song_by_nick("nobody".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_songleader_action_debug() {
        let action = SongleaderAction::Tempo {
            nick: "testuser".to_string(),
        };
        let debug = format!("{:?}", action);
        assert!(debug.contains("Tempo"));
        assert!(debug.contains("testuser"));
    }

    #[test]
    fn test_songleader_action_variants() {
        // Ensure all variants can be constructed
        let _request_url = SongleaderAction::RequestSongUrl {
            url: "https://example.com".to_string(),
            queued_by: "user".to_string(),
        };
        let _request_song = SongleaderAction::RequestSong {
            song: make_test_song("test"),
        };
        let _rm_id = SongleaderAction::RmSongById {
            id: "test".to_string(),
        };
        let _rm_nick = SongleaderAction::RmSongByNick {
            nick: "user".to_string(),
        };
        let _tempo = SongleaderAction::Tempo {
            nick: "user".to_string(),
        };
        let _bingo = SongleaderAction::Bingo {
            nick: "user".to_string(),
        };
        let _skal = SongleaderAction::Skål;
        let _list = SongleaderAction::ListSongs;
        let _force_tempo = SongleaderAction::ForceTempo;
        let _force_bingo = SongleaderAction::ForceBingo;
        let _force_singing = SongleaderAction::ForceSinging;
        let _pause = SongleaderAction::Pause;
        let _end = SongleaderAction::End;
        let _begin = SongleaderAction::Begin;
        let _help = SongleaderAction::Help;
    }

    #[test]
    fn test_mode_serialization() {
        // Test that Mode can be serialized (Inactive)
        let mode = Mode::Inactive;
        let json = serde_json::to_string(&mode).expect("Failed to serialize");
        let deserialized: Mode = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(mode, deserialized);

        // Test Singing mode
        let mode = Mode::Singing;
        let json = serde_json::to_string(&mode).expect("Failed to serialize");
        let deserialized: Mode = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(mode, deserialized);
    }
}
