//! Native application UI. Networking and platform services have independent owners.
mod controller;
pub mod network;
pub mod platform;
pub(crate) mod preferences;
mod storage;
mod terminal;
mod ui;
use nsh_core::prelude::*;
pub use ui::App;
pub fn root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<App>().tokens(
        nsh_base::tokens_of(Some(include_str!("ui/theme.json"))).expect("valid application tokens"),
    )
}
#[native_module("neko-remoted/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}
