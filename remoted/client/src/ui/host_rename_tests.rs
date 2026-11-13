use super::*;
use crate::{
    controller::{Client, Model, Phase},
    storage::HostAliases,
};
use nsh_core::{
    element::Root,
    platform::{Environment, Idiom, Locale, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

fn target() -> Binding {
    Binding {
        relay: neko_remoted_protocol::RelayAddress::parse("https://relay.example.com").unwrap(),
        route: neko_remoted_protocol::encode(&[1; 32]),
        host_key: neko_remoted_protocol::encode(&[2; 32]),
        device_id: "sidebar-device".into(),
    }
}

#[component]
fn Fixture() -> impl IntoElement {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let binding = signal(target());
    let ui = Ui::new(
        Model {
            phase: Phase::Offline,
            bindings: vec![target()],
            active_binding: Some(target()),
            remote_host_name: "server.local".into(),
            ..Model::default()
        },
        Client::fixture(sender),
    );
    let pending = signal(None::<String>);
    let unexpected = signal(false);
    let requests = signal(0);
    spawn(async move {
        while let Some(command) = receiver.recv().await {
            match command {
                Command::RenameBinding { binding, alias } => {
                    assert_eq!(binding, target());
                    requests.update(|count| *count += 1);
                    ui.model.update(|model| model.alias_saving = Some(binding));
                    pending.set(Some(alias));
                }
                _ => unexpected.set(true),
            }
        }
    });
    on_key_with(
        "ctrl+shift+y",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| {
            let alias = pending.get_untracked().unwrap();
            ui.model.update(|model| {
                model.aliases = if alias.is_empty() { HostAliases::default() } else {
                    let binding = target();
                    serde_json::from_value(serde_json::json!([{
                        "identity": { "relay": binding.relay, "route": binding.route, "host_key": binding.host_key },
                        "name": alias,
                    }])).unwrap()
                };
                model.alias_saving = None;
                model.error.clear();
            });
        },
    );
    on_key_with(
        "ctrl+shift+n",
        KeyOptions {
            when: nsh_core::keymap::When::Always,
            ..KeyOptions::default()
        },
        move |_| {
            ui.model.update(|model| {
                model.alias_saving = None;
                model.error = "主机别名未能确认保存，请重试；仍显示上次确认的名称。".into();
            });
        },
    );
    view()
        .flex_1()
        .child(device(
            ui,
            binding.read_only(),
            controls::session_row(
                controls::icons::Icon::Monitor,
                move || {
                    ui.model
                        .with(|model| host_names::display_name(model, &target()))
                },
                || true,
                || false,
                move || unexpected.set(true),
            ),
        ))
        .child(text(move || {
            format!(
                "rename-requests={} unexpected={}",
                requests.get(),
                unexpected.get()
            )
        }))
}

fn start(os: Os) -> Result<Headless, String> {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os,
            idiom: if os == Os::Android {
                Idiom::Phone
            } else {
                Idiom::Desktop
            },
            locale: Locale::parse("zh-CN").unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<Fixture>())
    })
    .map_err(|error| error.to_string())?;
    app.first_render()?;
    Ok(app)
}

#[test]
fn sidebar_right_click_renames_only_after_confirmation_and_empty_restores_hostname()
-> Result<(), String> {
    let mut app = start(Os::Linux)?;
    app.right_click("server.local")?;
    app.wait_for("重命名")?;
    app.press("重命名")?;
    app.type_text("主机别名", "  开发主机 🐈  ")?;
    app.press("保存别名")?;
    app.wait_for("rename-requests=1 unexpected=false")?;
    assert!(app.shows("server.local"));
    app.press_key("ctrl+shift+y")?;
    app.wait_for("开发主机 🐈")?;
    app.wait_until(
        "confirmed rename dialog dismissed",
        std::time::Duration::from_secs(2),
        |screen| screen.input_text("主机别名").ok().flatten().is_none(),
    )?;
    app.right_click("开发主机 🐈")?;
    app.press("重命名")?;
    app.clear("主机别名")?;
    app.press("保存别名")?;
    app.wait_for("rename-requests=2 unexpected=false")?;
    assert!(app.shows("开发主机 🐈"));
    app.press_key("ctrl+shift+y")?;
    app.wait_for("server.local")?;
    assert!(app.shows("rename-requests=2 unexpected=false"));
    Ok(())
}

#[test]
fn touch_overflow_retains_failed_alias_draft_and_retries_without_connecting() -> Result<(), String>
{
    let mut app = start(Os::Android)?;
    app.press("主机别名")?;
    app.press("重命名")?;
    app.type_text("主机别名", "keep this draft")?;
    app.press("保存别名")?;
    app.wait_for("rename-requests=1 unexpected=false")?;
    app.press_key("ctrl+shift+n")?;
    app.wait_for("主机别名未能确认保存，请重试；仍显示上次确认的名称。")?;
    assert!(app.shows("server.local"));
    assert_eq!(
        app.screen().input_text("主机别名")?,
        Some("keep this draft")
    );
    app.press("保存别名")?;
    app.wait_for("rename-requests=2 unexpected=false")?;
    app.press_key("ctrl+shift+y")?;
    app.wait_for("keep this draft")?;
    Ok(())
}
