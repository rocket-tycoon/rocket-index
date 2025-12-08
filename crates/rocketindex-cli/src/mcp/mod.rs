//!
//! This module exposes RocketIndex capabilities as MCP tools that AI assistants
//! can discover and use for code navigation.

#[cfg(test)]
mod tests;

pub mod config;
pub mod project_manager;
pub mod server;
pub mod tools;
pub mod watcher_pool;

pub use config::McpConfig;
pub use project_manager::ProjectManager;
// These are used by the server module internally
#[allow(unused_imports)]
pub use server::RocketIndexServer;
#[allow(unused_imports)]
pub use watcher_pool::WatcherPool;
