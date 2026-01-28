//! Unit tests for the songbook module

#[cfg(test)]
mod tests {
    use crate::songbook::SongbookSong;

    fn make_test_songbook_song(id: &str, title: Option<&str>, book: Option<&str>) -> SongbookSong {
        SongbookSong {
            id: id.to_string(),
            url: Some(format!("https://example.com/songs/{}", id)),
            title: title.map(|t| t.to_string()),
            book: book.map(|b| b.to_string()),
            queued_by: Some("testuser".to_string()),
        }
    }

    #[test]
    fn test_songbook_song_equality_by_id() {
        let song1 = make_test_songbook_song("song-1", Some("Title A"), Some("Book A"));
        let song2 = make_test_songbook_song("song-1", Some("Title B"), Some("Book B")); // Same ID
        let song3 = make_test_songbook_song("song-2", Some("Title A"), Some("Book A")); // Different ID

        assert_eq!(song1, song2);
        assert_ne!(song1, song3);
    }

    #[test]
    fn test_songbook_song_display_with_book() {
        let song = make_test_songbook_song("test-song", Some("Helan Går"), Some("TF Sångbok"));

        let display = format!("{}", song);
        assert_eq!(display, "Helan Går (TF Sångbok)");
    }

    #[test]
    fn test_songbook_song_display_without_book() {
        let song = SongbookSong {
            id: "test-id".to_string(),
            url: None,
            title: Some("Song Title".to_string()),
            book: None,
            queued_by: None,
        };

        let display = format!("{}", song);
        assert_eq!(display, "Song Title");
    }

    #[test]
    fn test_songbook_song_display_without_title() {
        let song = SongbookSong {
            id: "fallback-id".to_string(),
            url: None,
            title: None,
            book: None,
            queued_by: None,
        };

        let display = format!("{}", song);
        assert_eq!(display, "fallback-id");
    }

    #[test]
    fn test_songbook_song_default() {
        let song = SongbookSong::default();

        assert!(song.id.is_empty());
        assert!(song.url.is_none());
        assert!(song.title.is_none());
        assert!(song.book.is_none());
        assert!(song.queued_by.is_none());
    }

    #[test]
    fn test_songbook_song_clone() {
        let song = make_test_songbook_song("clone-test", Some("Original"), Some("Book 1"));
        let cloned = song.clone();

        assert_eq!(song.id, cloned.id);
        assert_eq!(song.url, cloned.url);
        assert_eq!(song.title, cloned.title);
        assert_eq!(song.book, cloned.book);
        assert_eq!(song.queued_by, cloned.queued_by);
    }

    #[test]
    fn test_songbook_song_serialization() {
        let song = make_test_songbook_song("serial-test", Some("Test Title"), Some("Test Book"));

        let json = serde_json::to_string(&song).expect("Failed to serialize");
        assert!(json.contains("serial-test"));
        assert!(json.contains("Test Title"));

        let deserialized: SongbookSong =
            serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(song.id, deserialized.id);
        assert_eq!(song.title, deserialized.title);
    }
}
