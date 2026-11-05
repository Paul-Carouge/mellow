//! Link previews' pure parts and fetcher (ADR-0041); the queue and the worker live with messages
//! (`domain::messages::link_previews`).

pub(crate) mod canonical_links;
pub(crate) mod fetcher;
pub(crate) mod links;
pub(crate) mod page;
pub(crate) mod providers;
pub(crate) mod public_addresses;
