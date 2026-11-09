//! Escritorio GTK de ComandOS: tablero WebKit, pestañas de terminal sobre tmux y popups.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]
pub mod agent_stop;
pub mod clipboard_bridge;
pub mod config;
pub mod dash_client;
pub mod feed;
pub mod fixture;
pub mod guard;
pub mod ipc;
pub mod jobs;
pub mod layout_dump;
pub mod notifyd;
pub mod poll;
pub mod proc;
pub mod restore;
pub mod resume;
pub mod snapshot;
pub mod state_files;
pub mod tab_actions;
pub mod tabs;
pub mod term;
pub mod theme;
pub mod tmux;
pub mod ui;
pub mod workspace_resize;
pub mod workspace_view;
