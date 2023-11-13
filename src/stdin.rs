use crate::{
    event::{self, EventBus},
    sources,
};
use tokio::io::AsyncReadExt;

#[allow(dead_code)]
pub fn init(bus: &EventBus) {
    let bus = bus.clone();
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut reader = tokio::io::BufReader::new(stdin);

        loop {
            let byte = reader.read_u8().await;

            match byte {
                Ok(b'r') => bus.send(event::Event::Symphonia(
                    sources::symphonia::SymphoniaAction::PlayFile {
                        file_path: "rickroll.m4a".to_string(),
                    },
                )),
                Ok(b'l') => bus.send(event::Event::TextToSpeech(
                    sources::espeak::TextToSpeechAction::Speak {
                        text: "Hello world".to_string(),
                        prio: sources::espeak::Priority::Low,
                    },
                )),
                Ok(b'h') => bus.send(event::Event::TextToSpeech(
                    sources::espeak::TextToSpeechAction::Speak {
                        text: "High prio".to_string(),
                        prio: sources::espeak::Priority::High,
                    },
                )),
                Ok(b'L') => {
                    let text = "Hello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello world".to_string();
                    bus.send(event::Event::TextToSpeech(
                        sources::espeak::TextToSpeechAction::Speak {
                            text,
                            prio: sources::espeak::Priority::High,
                        },
                    ))
                }
                // handle ctrl-c
                Ok(b'q' | 3) => {
                    info!("Received ctrl-c, exiting");
                    std::process::exit(0);
                }
                _ => {}
            }
        }
    });
}
