use tokio::io::AsyncReadExt;

use crate::{
    bus::{self, EventBus},
    sources,
};

pub fn start(bus: EventBus) {
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut reader = tokio::io::BufReader::new(stdin);

        loop {
            let byte = reader.read_u8().await.unwrap();

            match byte {
                b'l' => {
                    println!("Sending low prio tts event");
                    bus
                        .send(bus::Event::TextToSpeech(
                            sources::espeak::TextToSpeechAction::Speak {
                                text: "Hello world".to_string(),
                                prio: sources::espeak::Priority::Low,
                            },
                        ))
                        .unwrap();
                }
                b'h' => {
                    println!("Sending high prio tts event");
                    bus
                        .send(bus::Event::TextToSpeech(
                            sources::espeak::TextToSpeechAction::Speak {
                                text: "High prio".to_string(),
                                prio: sources::espeak::Priority::High,
                            },
                        ))
                        .unwrap();
                }
                b'L' => {
                    println!("Sending long tts event");
                    let text = "Hello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello worldHello world".to_string();
                    bus
                        .send(bus::Event::TextToSpeech(
                            sources::espeak::TextToSpeechAction::Speak {
                                text,
                                prio: sources::espeak::Priority::High,
                            },
                        ))
                        .unwrap();
                }
                // handle ctrl-c
                b'q' | 3 => {
                    println!("Received ctrl-c, exiting");
                    std::process::exit(0);
                }
                _ => {}
            }
        }
    });
}
