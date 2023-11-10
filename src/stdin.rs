use crate::{
    events::{self, EventBus},
    sources,
};

pub fn start(bus: EventBus) {
    tokio::spawn(async move {
        let stdin = std::io::stdin();
        let stdin = stdin.lock();
        let mut bytes = std::io::Read::bytes(stdin);
        loop {
            let b = bytes.next().unwrap().unwrap();
            match b {
                b'l' => {
                    bus.tx
                        .send(events::Event::TextToSpeech(
                            sources::espeak::TextToSpeechAction::Speak {
                                text: "Hello world".to_string(),
                                prio: sources::espeak::Priority::Low,
                            },
                        ))
                        .unwrap();
                }
                b'h' => {
                    bus.tx
                        .send(events::Event::TextToSpeech(
                            sources::espeak::TextToSpeechAction::Speak {
                                text: "High prio".to_string(),
                                prio: sources::espeak::Priority::High,
                            },
                        ))
                        .unwrap();
                }
                _ => {}
            }
        }
    });
}
