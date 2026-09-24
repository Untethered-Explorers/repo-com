#![forbid(unsafe_code)]
#![doc = "Thin command composition for the installed repo-com executable."]

#[path = "app.rs"]
pub mod app;

#[cfg(test)]
#[path = "../tests/command_routing_contract.rs"]
mod command_routing_contract;
