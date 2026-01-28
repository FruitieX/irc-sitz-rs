//! Unit tests for the event module

#[cfg(test)]
mod tests {
    use crate::event::{Event, EventBus};
    use crate::irc::IrcAction;
    use crate::playback::PlaybackAction;
    use std::time::Duration;

    #[test]
    fn test_event_bus_creation() {
        let bus = EventBus::new();
        // Should be able to subscribe
        let _subscriber = bus.subscribe();
    }

    #[test]
    fn test_event_bus_send_receive() {
        let bus = EventBus::new();
        let mut subscriber = bus.subscribe();

        // Send an event
        bus.send(Event::Irc(IrcAction::SendMsg("test message".to_string())));

        // Should be able to try_recv immediately (non-blocking)
        let result = subscriber.try_recv();
        assert!(result.is_ok());

        if let Event::Irc(IrcAction::SendMsg(msg)) = result.unwrap() {
            assert_eq!(msg, "test message");
        } else {
            panic!("Expected IrcAction::SendMsg");
        }
    }

    #[test]
    fn test_event_bus_multiple_subscribers() {
        let bus = EventBus::new();
        let mut sub1 = bus.subscribe();
        let mut sub2 = bus.subscribe();

        bus.send(Event::Playback(PlaybackAction::Play));

        // Both subscribers should receive the event
        assert!(sub1.try_recv().is_ok());
        assert!(sub2.try_recv().is_ok());
    }

    #[test]
    fn test_event_bus_empty_try_recv() {
        let bus = EventBus::new();
        let mut subscriber = bus.subscribe();

        // No events sent, try_recv should return an error
        let result = subscriber.try_recv();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_event_bus_async_recv() {
        let bus = EventBus::new();
        let mut subscriber = bus.subscribe();

        // Spawn a task to send an event after a small delay
        let bus_clone = bus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            bus_clone.send(Event::Playback(PlaybackAction::Pause));
        });

        // recv should block until the event is received
        let event = subscriber.recv().await;

        if let Event::Playback(PlaybackAction::Pause) = event {
            // Success!
        } else {
            panic!("Expected PlaybackAction::Pause");
        }
    }

    #[test]
    fn test_event_clone() {
        let event = Event::Irc(IrcAction::SendMsg("clone test".to_string()));
        let cloned = event.clone();

        if let Event::Irc(IrcAction::SendMsg(msg)) = cloned {
            assert_eq!(msg, "clone test");
        } else {
            panic!("Clone failed");
        }
    }

    #[test]
    fn test_event_debug() {
        let event = Event::Playback(PlaybackAction::Next);
        let debug = format!("{:?}", event);
        assert!(debug.contains("Playback"));
        assert!(debug.contains("Next"));
    }

    #[test]
    fn test_event_variants() {
        use crate::mixer::MixerAction;
        use crate::songleader::SongleaderAction;
        use crate::sources::espeak::{Priority, TextToSpeechAction};
        use crate::sources::symphonia::SymphoniaAction;

        // Ensure all Event variants can be constructed
        let _tts = Event::TextToSpeech(TextToSpeechAction::Speak {
            text: "test".to_string(),
            prio: Priority::Low,
        });
        let _mixer = Event::Mixer(MixerAction::DuckSecondaryChannels);
        let _symphonia = Event::Symphonia(SymphoniaAction::Stop);
        let _playback = Event::Playback(PlaybackAction::Play);
        let _irc = Event::Irc(IrcAction::SendMsg("test".to_string()));
        let _songleader = Event::Songleader(SongleaderAction::Help);
    }

    #[test]
    fn test_event_bus_clone() {
        let bus1 = EventBus::new();
        let bus2 = bus1.clone();

        let mut sub = bus1.subscribe();

        // Send via cloned bus
        bus2.send(Event::Playback(PlaybackAction::Prev));

        // Should receive via original subscriber
        assert!(sub.try_recv().is_ok());
    }
}
