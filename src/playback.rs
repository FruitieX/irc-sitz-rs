#[derive(Clone, Debug)]
pub struct Song {
    pub url: String,
    pub video_id: String,
    pub queued_by: String,
}

#[derive(Clone, Debug)]
pub enum PlaybackAction {
    Enqueue { song: Song },
    EndOfFile,
    ListQueue,
    Play,
    Pause,
    Prev,
    Next,
}