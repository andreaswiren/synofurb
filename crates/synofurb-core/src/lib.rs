//! synofurb-core: discovery, inspection, planning and installer engine.
pub mod addons;
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
pub mod hostkey;
pub mod manage;
pub mod offline;
pub mod recovery;
pub mod reset;
pub mod timing;
pub mod updater;
