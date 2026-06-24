//! # Event Channel
//!
//! A single `mpsc` channel carries *every* event the UI loop needs to process:
//! keyboard input, background-task progress, and periodic ticks.

use crossterm::event::{KeyEvent, MouseEvent};
use tokio::sync::mpsc;

/// Progress report sent by a background copy / move task.
#[derive(Debug, Clone)]
pub struct ProgressData {
    /// Human-readable operation name ("Copying", "Moving", "Extracting", …).
    pub operation:   &'static str,
    /// Base name of the file being processed (display only).
    pub source_name: String,
    pub bytes_done:  u64,
    pub bytes_total: u64,
    /// `true` on the final event — signals the event loop to close the popup.
    pub done:        bool,
}

/// Every event the application event loop can receive.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Progress(ProgressData),
    Tick,
}

pub type EventSender = mpsc::Sender<AppEvent>;
