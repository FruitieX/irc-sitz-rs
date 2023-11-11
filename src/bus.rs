use crate::irc::IrcAction;
use crate::playback::PlaybackAction;
use crate::songleader::SongleaderAction;
use crate::{
    mixer::MixerAction,
    sources::{espeak::TextToSpeechAction, symphonia::SymphoniaAction},
};
use tokio::sync::broadcast::{self, Sender};

pub type EventBus = Sender<Event>;

#[derive(Clone, Debug)]
pub enum Event {
    TextToSpeech(TextToSpeechAction),
    Mixer(MixerAction),
    Symphonia(SymphoniaAction),
    Playback(PlaybackAction),
    Irc(IrcAction),
    Songleader(SongleaderAction),
}

pub fn start() -> EventBus {
    let (tx, _rx) = broadcast::channel::<Event>(10);
    tx
}

pub fn debug(bus: &EventBus) {
    let bus = bus.clone();
    tokio::spawn(async move {
        let mut bus = bus.subscribe();
        loop {
            let event = bus.recv().await.unwrap();
            println!("Received event: {:?}", event);
        }
    });
}
