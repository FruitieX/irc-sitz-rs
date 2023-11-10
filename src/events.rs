use flume::{Receiver, Sender};

use crate::sources::espeak::TextToSpeechAction;

#[derive(Clone)]
pub struct EventBus {
    pub tx: Sender<Event>,
    pub rx: Receiver<Event>,
}

pub enum Event {
    TextToSpeech(TextToSpeechAction),
}

pub fn start() -> EventBus {
    let (tx, rx) = flume::unbounded::<Event>();
    EventBus { tx, rx }
}
