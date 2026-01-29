//! irc-sitz-rs library crate
//!
//! This module exposes internal types for integration testing.
//! The main binary is in main.rs.

#[macro_use]
extern crate log;

pub mod buffer;
pub mod config;
pub mod constants;
pub mod event;
#[cfg(feature = "irc")]
pub mod irc;
pub mod message;
pub mod mixer;
pub mod playback;
pub mod songbook;
pub mod songleader;
pub mod sources;
pub mod stdin;
pub mod youtube;

#[cfg(feature = "discord")]
pub mod discord;

// Test modules
#[cfg(test)]
mod buffer_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod songbook_tests;
#[cfg(test)]
mod songleader_tests;
