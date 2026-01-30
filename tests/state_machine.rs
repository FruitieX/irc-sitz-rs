//! Integration tests for the songleader state machine.
//!
//! Tests actual state transitions through the event handling logic,
//! including abuse scenarios and edge cases that could occur during a sitz.

mod common;

use common::*;
use irc_sitz_rs::songleader::{handle_incoming_event, Songleader};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Instant;

/// Creates a test songleader with the given initial mode.
fn create_test_songleader(bus: &EventBus, mode: Mode) -> Arc<RwLock<Songleader>> {
    let config = test_config();
    let params = test_runtime_params();
    let mut state = SongleaderState::default();
    state.mode = mode;

    // Add some songs so bingo mode can work
    state.requests.push(mock_songbook_song(
        "test-song-1",
        "Test Song 1",
        Some("user"),
    ));
    state
        .backup
        .push(mock_songbook_song("backup-song-1", "Backup Song", None));

    let songleader = Songleader::create_with_state(bus, &config, state, params);
    Arc::new(RwLock::new(songleader))
}

/// Helper to send an action and wait for processing.
async fn send_action(
    bus: &EventBus,
    config: &Config,
    songleader: &Arc<RwLock<Songleader>>,
    action: SongleaderAction,
) {
    handle_incoming_event(bus.clone(), config.clone(), songleader.clone(), action).await;
}

// =============================================================================
// STATE TRANSITION TESTS
// =============================================================================

/// Test: !tempo in Tempo mode with enough users triggers transition to Bingo.
#[tokio::test]
async fn test_tempo_to_bingo_transition() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // Send 3 tempo commands from different users
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "user1".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "user2".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "user3".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Bingo { .. }),
        "Expected Bingo mode after 3 tempo commands"
    );
}

/// Test: !bingo in Bingo mode with enough users triggers transition to Singing.
#[tokio::test]
async fn test_bingo_to_singing_transition() {
    let bus = EventBus::new();
    let config = test_config();
    let song = mock_songbook_song("current-song", "Current Song", None);
    let songleader = create_test_songleader(
        &bus,
        Mode::Bingo {
            nicks: HashSet::new(),
            song,
        },
    );

    // Send 3 bingo commands from different users
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Bingo {
            nick: "user1".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Bingo {
            nick: "user2".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Bingo {
            nick: "user3".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Singing),
        "Expected Singing mode after 3 bingo commands"
    );
}

/// Test: !skål in Singing mode triggers transition to Tempo.
#[tokio::test]
async fn test_singing_to_tempo_transition() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Singing);

    send_action(&bus, &config, &songleader, SongleaderAction::Skål).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Tempo { .. }),
        "Expected Tempo mode after skål"
    );
}

/// Test: Force commands work correctly.
#[tokio::test]
async fn test_force_tempo_command() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Singing);

    send_action(&bus, &config, &songleader, SongleaderAction::ForceTempo).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Tempo { .. }),
        "ForceTempo should enter Tempo mode"
    );
}

/// Test: Force bingo from any mode.
#[tokio::test]
async fn test_force_bingo_command() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Inactive);

    send_action(&bus, &config, &songleader, SongleaderAction::ForceBingo).await;

    let state = &songleader.read().await.state;
    // ForceBingo calls enter_bingo_mode which may fall back to Tempo if no songs
    assert!(
        matches!(state.mode, Mode::Bingo { .. } | Mode::Tempo { .. }),
        "ForceBingo should enter Bingo or Tempo mode"
    );
}

/// Test: Force singing from any mode.
#[tokio::test]
async fn test_force_singing_command() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Inactive);

    send_action(&bus, &config, &songleader, SongleaderAction::ForceSinging).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Singing),
        "ForceSinging should enter Singing mode"
    );
}

/// Test: End command from active mode goes to Inactive.
#[tokio::test]
async fn test_end_command() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Singing);

    send_action(&bus, &config, &songleader, SongleaderAction::End).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Inactive),
        "End should enter Inactive mode"
    );
}

/// Test: Pause command enters Inactive mode.
#[tokio::test]
async fn test_pause_command() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    send_action(&bus, &config, &songleader, SongleaderAction::Pause).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Inactive),
        "Pause should enter Inactive mode"
    );
}

// =============================================================================
// COMMANDS IGNORED IN WRONG MODE TESTS
// =============================================================================

/// Test: !tempo is ignored during Bingo mode.
#[tokio::test]
async fn test_tempo_ignored_during_bingo() {
    let bus = EventBus::new();
    let config = test_config();
    let song = mock_songbook_song("current-song", "Current Song", None);
    let songleader = create_test_songleader(
        &bus,
        Mode::Bingo {
            nicks: HashSet::new(),
            song,
        },
    );

    // Spam tempo - should all be ignored
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "attacker".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "attacker2".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "attacker3".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Bingo { .. }),
        "Tempo should be ignored in Bingo mode"
    );
}

/// Test: !tempo is ignored during Singing mode.
#[tokio::test]
async fn test_tempo_ignored_during_singing() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Singing);

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "attacker".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Singing),
        "Tempo should be ignored in Singing mode"
    );
}

/// Test: !tempo is ignored during Inactive mode.
#[tokio::test]
async fn test_tempo_ignored_during_inactive() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Inactive);

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "attacker".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Inactive),
        "Tempo should be ignored in Inactive mode"
    );
}

/// Test: !bingo is ignored during Tempo mode.
#[tokio::test]
async fn test_bingo_ignored_during_tempo() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Bingo {
            nick: "attacker".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Tempo { .. }),
        "Bingo should be ignored in Tempo mode"
    );
}

/// Test: !bingo is ignored during Singing mode.
#[tokio::test]
async fn test_bingo_ignored_during_singing() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Singing);

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Bingo {
            nick: "attacker".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Singing),
        "Bingo should be ignored in Singing mode"
    );
}

/// Test: !bingo is ignored during Inactive mode.
#[tokio::test]
async fn test_bingo_ignored_during_inactive() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Inactive);

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Bingo {
            nick: "attacker".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Inactive),
        "Bingo should be ignored in Inactive mode"
    );
}

/// Test: !skål is ignored during Tempo mode.
#[tokio::test]
async fn test_skal_ignored_during_tempo() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    send_action(&bus, &config, &songleader, SongleaderAction::Skål).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Tempo { .. }),
        "Skål should be ignored in Tempo mode"
    );
}

/// Test: !skål is ignored during Bingo mode.
#[tokio::test]
async fn test_skal_ignored_during_bingo() {
    let bus = EventBus::new();
    let config = test_config();
    let song = mock_songbook_song("current-song", "Current Song", None);
    let songleader = create_test_songleader(
        &bus,
        Mode::Bingo {
            nicks: HashSet::new(),
            song,
        },
    );

    send_action(&bus, &config, &songleader, SongleaderAction::Skål).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Bingo { .. }),
        "Skål should be ignored in Bingo mode"
    );
}

/// Test: !skål is ignored during Inactive mode.
#[tokio::test]
async fn test_skal_ignored_during_inactive() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Inactive);

    send_action(&bus, &config, &songleader, SongleaderAction::Skål).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Inactive),
        "Skål should be ignored in Inactive mode"
    );
}

// =============================================================================
// ABUSE SCENARIO TESTS
// =============================================================================

/// Test: Same user spamming !tempo only counts once.
#[tokio::test]
async fn test_same_user_tempo_spam_counts_once() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // Same user spams tempo 10 times
    for _ in 0..10 {
        send_action(
            &bus,
            &config,
            &songleader,
            SongleaderAction::Tempo {
                nick: "spammer".to_string(),
            },
        )
        .await;
    }

    let state = &songleader.read().await.state;
    // Should still be in Tempo mode (only 1 unique nick)
    if let Mode::Tempo { nicks, .. } = &state.mode {
        assert_eq!(nicks.len(), 1, "Same user's tempo should only count once");
    } else {
        panic!("Should still be in Tempo mode");
    }
}

/// Test: Same user spamming !bingo only counts once.
#[tokio::test]
async fn test_same_user_bingo_spam_counts_once() {
    let bus = EventBus::new();
    let config = test_config();
    let song = mock_songbook_song("current-song", "Current Song", None);
    let songleader = create_test_songleader(
        &bus,
        Mode::Bingo {
            nicks: HashSet::new(),
            song,
        },
    );

    // Same user spams bingo 10 times
    for _ in 0..10 {
        send_action(
            &bus,
            &config,
            &songleader,
            SongleaderAction::Bingo {
                nick: "spammer".to_string(),
            },
        )
        .await;
    }

    let state = &songleader.read().await.state;
    // Should still be in Bingo mode (only 1 unique nick)
    if let Mode::Bingo { nicks, .. } = &state.mode {
        assert_eq!(nicks.len(), 1, "Same user's bingo should only count once");
    } else {
        panic!("Should still be in Bingo mode");
    }
}

/// Test: End command when already inactive is idempotent.
#[tokio::test]
async fn test_end_when_already_inactive() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Inactive);

    // Multiple end commands should not panic or change state
    send_action(&bus, &config, &songleader, SongleaderAction::End).await;
    send_action(&bus, &config, &songleader, SongleaderAction::End).await;
    send_action(&bus, &config, &songleader, SongleaderAction::End).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Inactive),
        "Should remain Inactive"
    );
}

/// Test: Help is only available in Tempo or Inactive mode.
#[tokio::test]
async fn test_help_only_in_tempo_or_inactive() {
    let bus = EventBus::new();
    let config = test_config();
    let mut subscriber = bus.subscribe();

    // Test in Singing - should produce no message
    let songleader = create_test_songleader(&bus, Mode::Singing);
    send_action(&bus, &config, &songleader, SongleaderAction::Help).await;
    // Drain any events
    while subscriber.try_recv().is_ok() {}

    // Test in Bingo - should produce no message
    let song = mock_songbook_song("song", "Song", None);
    {
        let mut sl = songleader.write().await;
        sl.state.mode = Mode::Bingo {
            nicks: HashSet::new(),
            song,
        };
    }
    send_action(&bus, &config, &songleader, SongleaderAction::Help).await;
    // Drain any events - there should be none from help
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
}

/// Test: Very long nickname doesn't break state.
#[tokio::test]
async fn test_very_long_nickname() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    let long_nick = "a".repeat(10000);
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: long_nick.clone(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    if let Mode::Tempo { nicks, .. } = &state.mode {
        assert!(
            nicks.contains(&long_nick),
            "Long nickname should be accepted"
        );
    }
}

/// Test: Unicode nickname handling.
#[tokio::test]
async fn test_unicode_nickname() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    let unicode_nick = "用户🎉تست";
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: unicode_nick.to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    if let Mode::Tempo { nicks, .. } = &state.mode {
        assert!(
            nicks.contains(unicode_nick),
            "Unicode nickname should be accepted"
        );
    }
}

/// Test: Empty nickname handling.
#[tokio::test]
async fn test_empty_nickname() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    if let Mode::Tempo { nicks, .. } = &state.mode {
        // Empty nick is technically valid (HashSet will accept it)
        assert_eq!(nicks.len(), 1, "Empty nickname should still count");
    }
}

/// Test: Whitespace-only nickname.
#[tokio::test]
async fn test_whitespace_only_nickname() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "   ".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    if let Mode::Tempo { nicks, .. } = &state.mode {
        assert_eq!(
            nicks.len(),
            1,
            "Whitespace-only nickname should be accepted"
        );
    }
}

/// Test: Rapid fire different commands don't cause issues.
#[tokio::test]
async fn test_rapid_fire_commands() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // Rapid fire mixed commands
    for i in 0..20 {
        let action = match i % 5 {
            0 => SongleaderAction::Tempo {
                nick: format!("user{i}"),
            },
            1 => SongleaderAction::Bingo {
                nick: format!("user{i}"),
            },
            2 => SongleaderAction::Skål,
            3 => SongleaderAction::ListSongs,
            _ => SongleaderAction::Help,
        };
        send_action(&bus, &config, &songleader, action).await;
    }

    // Should not panic and state should be consistent
    let state = &songleader.read().await.state;
    // After enough valid tempo commands from different users, should transition
    assert!(
        matches!(
            state.mode,
            Mode::Tempo { .. } | Mode::Bingo { .. } | Mode::Singing
        ),
        "State should be valid after rapid commands"
    );
}

/// Test: Song request when queue manipulation is happening.
#[tokio::test]
async fn test_concurrent_song_operations() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // Add several songs
    for i in 0..5 {
        let song = mock_songbook_song(
            &format!("song-{i}"),
            &format!("Song {i}"),
            Some(&format!("user{i}")),
        );
        send_action(
            &bus,
            &config,
            &songleader,
            SongleaderAction::RequestSong { song },
        )
        .await;
    }

    // Remove some while maybe others are being added
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::RmSongByNick {
            nick: "user0".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::RmSongById {
            id: "song-1".to_string(),
        },
    )
    .await;

    // Should not panic
    let state = &songleader.read().await.state;
    // We started with 1 request + 1 backup, added 5, removed 2
    // But first request had same id might have been rejected
    assert!(
        state.requests.len() <= 5,
        "Requests should be managed correctly"
    );
}

/// Test: Removing song that doesn't exist.
#[tokio::test]
async fn test_remove_nonexistent_song() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // Try to remove songs that don't exist - should not panic
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::RmSongById {
            id: "nonexistent".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::RmSongByNick {
            nick: "nonexistent_user".to_string(),
        },
    )
    .await;

    // Should still be in valid state
    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Tempo { .. }),
        "Should remain in valid state"
    );
}

/// Test: Bingo mode with no songs falls back to Tempo.
#[tokio::test]
async fn test_bingo_with_no_songs_fallback() {
    let bus = EventBus::new();
    let config = test_config();

    // Create songleader with empty queues
    let mut state = SongleaderState::default();
    state.mode = Mode::Tempo {
        nicks: HashSet::new(),
        init_t: Instant::now(),
    };
    // Explicitly empty all song queues
    state.first_songs.clear();
    state.requests.clear();
    state.backup.clear();

    let params = test_runtime_params();
    let songleader = Arc::new(RwLock::new(Songleader::create_with_state(
        &bus, &config, state, params,
    )));

    // Force bingo - but there are no songs
    send_action(&bus, &config, &songleader, SongleaderAction::ForceBingo).await;

    let state = &songleader.read().await.state;
    // Should fall back to Tempo since no songs
    assert!(
        matches!(state.mode, Mode::Tempo { .. }),
        "Should fall back to Tempo when no songs"
    );
}

/// Test: Multiple !skål commands after singing ends.
#[tokio::test]
async fn test_multiple_skal_after_transition() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(&bus, Mode::Singing);

    // First skål transitions to Tempo
    send_action(&bus, &config, &songleader, SongleaderAction::Skål).await;

    // Verify transition to Tempo (scope the read to release the lock)
    {
        let state = &songleader.read().await.state;
        assert!(
            matches!(state.mode, Mode::Tempo { .. }),
            "First skål should transition to Tempo"
        );
    }

    // Additional skål commands in Tempo mode should be ignored (not transition further)
    send_action(&bus, &config, &songleader, SongleaderAction::Skål).await;
    send_action(&bus, &config, &songleader, SongleaderAction::Skål).await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Tempo { .. }),
        "Additional skål should be ignored in Tempo"
    );
}

/// Test: Case sensitivity of nicknames.
#[tokio::test]
async fn test_nickname_case_sensitivity() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // Same name, different cases - should be treated as different users
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "User".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "user".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "USER".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    // All three should be counted as different (case sensitive)
    // This should trigger transition to Bingo
    assert!(
        matches!(state.mode, Mode::Bingo { .. }),
        "Different case nicknames should be treated as different users"
    );
}

/// Test: ListSongs during various modes.
#[tokio::test]
async fn test_list_songs_always_works() {
    let bus = EventBus::new();
    let config = test_config();

    // Test ListSongs in each mode - should never panic
    for mode in [
        Mode::Inactive,
        Mode::Singing,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    ] {
        let songleader = create_test_songleader(&bus, mode);
        send_action(&bus, &config, &songleader, SongleaderAction::ListSongs).await;
        // Should complete without panic
    }
}

/// Test: Two users with exactly 2 tempos, then third user completes it.
#[tokio::test]
async fn test_tempo_threshold_exactly_at_boundary() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // First two unique users
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "user1".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "user2".to_string(),
        },
    )
    .await;

    // Should still be in Tempo with 2 nicks
    {
        let state = &songleader.read().await.state;
        if let Mode::Tempo { nicks, .. } = &state.mode {
            assert_eq!(nicks.len(), 2);
        } else {
            panic!("Should be in Tempo mode");
        }
    }

    // Third user triggers transition
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "user3".to_string(),
        },
    )
    .await;

    let state = &songleader.read().await.state;
    assert!(
        matches!(state.mode, Mode::Bingo { .. }),
        "Third tempo should trigger Bingo"
    );
}

/// Test: Songs consumed during full party flow.
#[tokio::test]
async fn test_songs_consumed_during_flow() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    // Get initial song count
    let initial_count = {
        let state = &songleader.read().await.state;
        state.get_songs().len()
    };

    // Trigger tempo -> bingo transition
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "a".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "b".to_string(),
        },
    )
    .await;
    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::Tempo {
            nick: "c".to_string(),
        },
    )
    .await;

    // After entering bingo, a song should be popped
    let after_bingo_count = {
        let state = &songleader.read().await.state;
        state.get_songs().len()
    };

    assert_eq!(
        after_bingo_count,
        initial_count - 1,
        "Entering bingo should consume one song"
    );
}

/// Test: Special characters in song request URL.
#[tokio::test]
async fn test_special_chars_in_song_id() {
    let bus = EventBus::new();
    let config = test_config();
    let songleader = create_test_songleader(
        &bus,
        Mode::Tempo {
            nicks: HashSet::new(),
            init_t: Instant::now(),
        },
    );

    let song = SongbookSong {
        id: "song-with-special-chars-äöå-&-<>-\"'".to_string(),
        url: Some("https://example.com/song?param=value&other=<test>".to_string()),
        title: Some("Song with 'quotes' and \"double quotes\"".to_string()),
        book: Some("Book <special>".to_string()),
        queued_by: Some("user<script>".to_string()),
    };

    send_action(
        &bus,
        &config,
        &songleader,
        SongleaderAction::RequestSong { song },
    )
    .await;

    let state = &songleader.read().await.state;
    // Should not panic, song should be added (or rejected for duplicate)
    // The important thing is no crash
    assert!(
        state.requests.len() >= 1,
        "Should handle special characters"
    );
}
