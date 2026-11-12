mod composer;
mod connection;
mod controls;
mod file_view;
mod files;
pub(crate) mod herdr;
#[cfg(not(target_os = "android"))]
mod hosting;
mod navigation;
mod preferences;
mod resources;
mod session_list;
mod shell;
mod sidebar;
mod state;
mod terminal_colors;
mod terminal_input;
mod terminal_observer;
mod terminal_surface;
mod terminal_view;
#[cfg(test)]
mod tests;
pub(crate) mod transfers;
#[cfg(all(test, target_os = "linux"))]
mod visual_tests;
mod workspace;
mod workspace_header;

use nsh_base::{nav_page, nav_stack, theme};
use nsh_core::prelude::*;

#[component]
pub fn App() -> impl IntoElement {
    #[cfg(not(target_os = "android"))]
    crate::platform::start_services();
    let startup = (|| -> anyhow::Result<_> {
        let display = crate::preferences::Ui::start()?;
        let terminal = crate::terminal::initialize()?;
        Ok((display, terminal))
    })();
    let (_display, terminal) = match startup {
        Ok(startup) => startup,
        Err(error) => {
            return view()
                .p_4()
                .gap_3()
                .bg(move || theme().background)
                .child(
                    text(move || crate::preferences::tr("无法启动终端"))
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(
                    text(error.to_string())
                        .select_text()
                        .text_color(move || theme().destructive),
                )
                .into_element();
        }
    };
    provide_context(terminal);
    let ui = state::Ui::start();
    provide_context(ui);
    frame().into_element()
}

/// Shared production navigation frame; startup services are owned by App.
fn frame() -> impl IntoElement {
    let ui = use_context::<state::Ui>().expect("App provides the native UI controller context");
    nav_stack()
        .flex_1()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(
            nav_page()
                .title("neoko-terminal")
                .header_visible(false)
                .pop_on_back(move || !ui.secondary())
                .on_back_press(move || ui.back())
                .child(shell::body()),
        )
        .into_element()
}
