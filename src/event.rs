use crate::irc::IrcAction;
use crate::playback::PlaybackAction;
use crate::songleader::SongleaderAction;
use crate::{
    mixer::MixerAction,
    sources::{espeak::TextToSpeechAction, symphonia::SymphoniaAction},
};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::broadcast::{self, Receiver, Sender};

#[derive(Clone)]
pub struct EventBus {
    tx: Sender<Event>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel::<Event>(100);
        Self { tx }
    }
    pub fn send(&self, event: Event) {
        let result = self.tx.send(event);

        if let Err(e) = result {
            error!("Error while sending event: {:?}", e);
        }
    }

    pub fn subscribe(&self) -> Subscriber {
        Subscriber::new(self.tx.subscribe())
    }
}

pub struct Subscriber {
    rx: Receiver<Event>,
}

impl Subscriber {
    pub fn new(rx: Receiver<Event>) -> Self {
        Self { rx }
    }

    pub fn try_recv(&mut self) -> Result<Event, TryRecvError> {
        self.rx.try_recv()
    }

    pub async fn recv(&mut self) -> Event {
        loop {
            let event = self.rx.recv().await;

            match event {
                Ok(event) => break event,
                Err(RecvError::Closed) => {
                    panic!("Tried to read recv from EventBus with all sender halves dropped, this should never happen")
                }
                Err(RecvError::Lagged(skipped)) => {
                    warn!(
                        "EventBus::Subscriber lagging behind senders, skipping {skipped} messages"
                    );
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    TextToSpeech(TextToSpeechAction),
    Mixer(MixerAction),
    Symphonia(SymphoniaAction),
    Playback(PlaybackAction),
    Irc(IrcAction),
    Songleader(SongleaderAction),
}

pub fn debug(bus: &EventBus) {
    let bus = bus.clone();
    tokio::spawn(async move {
        let mut bus = bus.subscribe();
        loop {
            let event = bus.recv().await;
            if matches!(
                event,
                Event::Playback(PlaybackAction::PlaybackProgress { .. })
            ) {
                trace!("Received event: {:?}", event);
            } else {
                debug!("Received event: {:?}", event);
            }
        }
    });
}
