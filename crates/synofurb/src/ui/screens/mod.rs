//! One module per screen; each adds `App::page_<name>` (see `render.rs` for how a [`Page`](super::render::Page) is laid out).

pub mod about;
pub mod bootmgr;
pub mod confirm;
pub mod connect;
pub mod done;
pub mod failed;
pub mod goal;
pub mod help;
pub mod inspect;
pub mod install;
pub mod inventory;
pub mod logviewer;
pub mod manage;
pub mod plan;
pub mod recover;
pub mod settings;
pub mod system;
pub mod update;
pub mod update_status;
pub mod welcome;
