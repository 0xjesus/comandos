//! Terminal de escritorio sobre `comandos-term`: motor, pintado, PTY y widget.
pub mod engine;
pub mod paint;

pub mod pty;
pub mod settle;

pub mod keys;
pub mod lag;
pub mod noise;
pub mod pick;
pub mod tiles;
pub mod links;
pub mod hover;
pub mod schedule;

pub mod lifecycle;
pub mod view;

pub mod shadow;
pub mod control;
