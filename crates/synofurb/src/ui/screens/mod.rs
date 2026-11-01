//! One module per screen. Each adds a `page_<name>` method to [`App`](super::app::App) that returns a
//! [`Page`](super::render::Page): title, body, and the action-bar buttons. To add a screen, see docs/GUI.md.

mod bootmgr;
mod confirm;
mod connect;
mod done;
mod failed;
mod goal;
mod inspect;
mod install;
mod inventory;
pub mod logviewer;
mod plan;
mod recover;
mod settings;
mod system;
pub mod update;
