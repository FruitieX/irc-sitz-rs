use crate::{mixer::MixerAction, sources::espeak::TextToSpeechAction};
use tokio::sync::broadcast::{self, Sender};

pub type EventBus = Sender<Event>;

#[derive(Clone, Debug)]
pub enum Event {
    TextToSpeech(TextToSpeechAction),
    Mixer(MixerAction),
}

pub fn start() -> EventBus {
    let (tx, _rx) = broadcast::channel::<Event>(10);
    tx
}
