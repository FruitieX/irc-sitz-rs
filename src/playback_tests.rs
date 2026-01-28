//! Unit tests for the playback module

#[cfg(test)]
mod tests {
    use crate::playback::{PlaybackAction, Song};

    /// Creates a test song with default values
    fn make_test_song(id: &str, queued_by: &str) -> Song {
        Song {
            id: id.to_string(),
            url: format!("https://youtu.be/{}", id),
            title: format!("Test Song {}", id),
            channel: "Test Channel".to_string(),
            duration: 180, // 3 minutes
            queued_by: queued_by.to_string(),
        }
    }

    #[test]
    fn test_song_equality_by_id() {
        let song1 = make_test_song("abc123", "user1");
        let song2 = make_test_song("abc123", "user2"); // Different user, same ID
        let song3 = make_test_song("xyz789", "user1"); // Same user, different ID

        // Songs are equal if they have the same ID
        assert_eq!(song1, song2);
        // Songs with different IDs are not equal
        assert_ne!(song1, song3);
    }

    #[test]
    fn test_song_clone() {
        let song = make_test_song("test123", "testuser");
        let cloned = song.clone();

        assert_eq!(song.id, cloned.id);
        assert_eq!(song.url, cloned.url);
        assert_eq!(song.title, cloned.title);
        assert_eq!(song.channel, cloned.channel);
        assert_eq!(song.duration, cloned.duration);
        assert_eq!(song.queued_by, cloned.queued_by);
    }

    #[test]
    fn test_playback_action_debug() {
        let action = PlaybackAction::Enqueue {
            song: make_test_song("test", "user"),
        };
        // Ensure Debug is implemented and doesn't panic
        let debug_str = format!("{:?}", action);
        assert!(debug_str.contains("Enqueue"));
    }

    #[test]
    fn test_playback_action_variants() {
        // Test that all action variants can be constructed
        let _enqueue = PlaybackAction::Enqueue {
            song: make_test_song("test", "user"),
        };
        let _end = PlaybackAction::EndOfSong;
        let _list = PlaybackAction::ListQueue { offset: Some(5) };
        let _list_none = PlaybackAction::ListQueue { offset: None };
        let _rm_pos = PlaybackAction::RmSongByPos { pos: 3 };
        let _rm_nick = PlaybackAction::RmSongByNick {
            nick: "testuser".to_string(),
        };
        let _play = PlaybackAction::Play;
        let _pause = PlaybackAction::Pause;
        let _prev = PlaybackAction::Prev;
        let _next = PlaybackAction::Next;
        let _progress = PlaybackAction::PlaybackProgress { position: 120 };
    }

    #[test]
    fn test_song_serialization() {
        let song = make_test_song("serialize_test", "serializer");

        // Test that serialization works
        let json = serde_json::to_string(&song).expect("Failed to serialize song");
        assert!(json.contains("serialize_test"));
        assert!(json.contains("serializer"));

        // Test that deserialization works
        let deserialized: Song = serde_json::from_str(&json).expect("Failed to deserialize song");
        assert_eq!(song.id, deserialized.id);
        assert_eq!(song.queued_by, deserialized.queued_by);
    }
}
