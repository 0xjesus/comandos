//! Portable ComandOS domain rules. No I/O, clocks, environment or subprocesses.
pub mod event;
pub mod hook;
pub mod turn;

pub mod json;
/// JSON truthiness at the existing Python compatibility boundary.
pub use json::truthy as legacy_truthy;

pub mod notifications;

pub mod workspace;

pub mod focus;
pub mod pomodoro;

pub mod mcp_descriptions;

pub mod allocation;
pub mod chat_markdown;
pub mod analytics_week;

pub mod work_marks;

pub mod dashboard_access;

pub mod malloc_tuning;
pub mod repo;
pub mod text;
pub mod usage_state;

pub mod web_assets;
