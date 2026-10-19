//! Client logic shared by the Mellow desktop and Android apps (ADR-0011): connection to one
//! instance, state, and sync. No UI code lives here.

mod error;
mod gateway;
mod instance;
mod rest;
pub mod state;

pub use error::ClientError;
pub use gateway::{GatewayUpdate, run_gateway};
pub use instance::InstanceAddress;
pub use rest::RestClient;
