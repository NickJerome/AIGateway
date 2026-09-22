//! Plugin gateway components. See docs/rust-architecture.md for ownership and adapter boundaries.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod bytecode;
pub mod config;
pub mod durable;
pub mod engine;
pub mod http;
pub mod identity;
pub mod ir;
pub mod limits;
pub mod network;
pub mod plugins;
pub mod runtime;
pub mod scheduled;
pub mod storage;
pub mod store;
pub mod telemetry;
pub mod web;
pub mod websocket;
pub mod worker;
