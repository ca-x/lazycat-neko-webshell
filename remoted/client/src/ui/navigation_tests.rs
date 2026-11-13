use super::*;
use crate::{
    controller::{Client, Model},
    preferences::{Preferences, Ui as Settings, catalog::Catalog},
    ui::state::Ui,
};
use nsh_core::{
    element::Root,
    platform::{Environment, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[component]
fn Fixture() -> impl IntoElement {
    let mut values = Preferences::defaults(Catalog::shared().unwrap());
    values.default_backend = SessionBackend::Herdr;
    let settings = Settings::from_loaded(values).unwrap();
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    let ui = Ui::new(Model::default(), Client::fixture(sender));
    install_startup_selection(ui, settings);
    view()
        .child(text(move || {
            format!(
                "selected:{}:{:?}",
                if ui.navigation.section.get() == Section::Herdr {
                    "herdr"
                } else {
                    "terminal"
                },
                ui.navigation.ordinary_backend.get()
            )
        }))
        .child(
            nsh_base::button()
                .on_press(move || ui.navigation.section.set(Section::Terminal))
                .child(text("select ordinary")),
        )
        .child(
            nsh_base::button()
                .on_press(move || {
                    ui.model
                        .update(|model| model.error = "unrelated status".into())
                })
                .child(text("status update")),
        )
        .child(
            nsh_base::button()
                .on_press(move || ui.model.update(|model| model.local = true))
                .child(text("enter local")),
        )
        .child(
            nsh_base::button()
                .on_press(move || ui.navigation.ordinary_backend.set(SessionBackend::Hc))
                .child(text("select hc")),
        )
}

#[test]
fn initial_default_selects_once_and_local_reentry_applies_it_without_overriding_manual_navigation()
-> Result<(), String> {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os: Os::Linux,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<Fixture>())
    })
    .map_err(|error| error.to_string())?;
    app.wait_for("text:selected:herdr:Native")?;
    app.press("select ordinary")?;
    app.wait_for("text:selected:terminal:Native")?;
    app.press("status update")?;
    app.wait_for("text:selected:terminal:Native")?;
    app.press("enter local")?;
    app.wait_for("text:selected:herdr:Native")?;
    app.press("select ordinary")?;
    app.press("select hc")?;
    app.press("status update")?;
    app.wait_for("text:selected:terminal:Hc")?;
    Ok(())
}
