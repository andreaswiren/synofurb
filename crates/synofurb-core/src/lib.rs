//! synofurb-core: discovery, inspection, planning and installer engine.
pub mod config;
pub mod debian;
pub mod discovery;
pub mod inspect;
pub mod kexec;
pub mod logs;
pub mod model;
pub mod payload;
pub mod plan;
pub mod platform;
pub mod remote;
pub mod script;
pub mod server;
pub mod session;

pub mod compat;
pub mod diskbackup;
pub mod domboot;
pub mod offline;
pub mod updater;
pub mod manage;
