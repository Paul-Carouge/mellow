//! The Mellow server. Modules follow the dependency direction transport → domain → storage (ARCHITECTURE.md §4.1).

pub mod commands;
pub mod config;
pub mod domain;
pub(crate) mod http;
pub mod storage;
