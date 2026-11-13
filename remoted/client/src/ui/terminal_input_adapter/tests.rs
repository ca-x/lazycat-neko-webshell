use super::*;
use editing::{HEADER_BYTES, HISTORY_LIMIT, RETAINED_BYTES, native_parts};

fn native(body: &str) -> String {
    format!("{}{body}", Editing::initial_value())
}

#[test]
fn android_candidates_commit_once_and_cancellation_restores_the_previous_value() {
    let mut edit = Editing::default();
    assert_eq!(edit.composition(true), "");
    assert_eq!(edit.change(&native("n")), "");
    assert_eq!(edit.change(&native("ni")), "");
    assert_eq!(edit.change(&native("你")), "");
    assert_eq!(edit.composition(false), "你");
    assert_eq!(edit.composition(false), "");
    assert_eq!(edit.change(&native("你")), "");
    edit.composition(true);
    edit.change(&native("你n"));
    edit.change(&native("你ni"));
    edit.change(&native("你"));
    assert_eq!(edit.composition(false), "");
    edit.composition(true);
    edit.change(&native("你你"));
    assert_eq!(edit.composition(false), "你");
}

#[test]
fn desktop_preedit_outside_the_native_value_only_sends_the_final_commit() {
    let mut edit = Editing::default();
    assert_eq!(edit.change(&native("echo ")), "echo ");
    edit.composition(true);
    edit.composition(true);
    assert_eq!(edit.change(&native("echo 中文e\u{301}🙂")), "");
    assert_eq!(edit.composition(false), "中文e\u{301}🙂");
    edit.composition(true);
    assert_eq!(edit.composition(false), "");
    assert_eq!(edit.change(&native("echo 中文e\u{301}🙂!")), "!");
}

use crate::{
    controller::{Client, Command, Model, Pane, PaneKind, Phase},
    ui::state::Ui,
};
use neko_remoted_protocol::{
    decode,
    messages::{Operation, Permission},
};
use nsh_core::{
    EventKind, EventPayload, PayloadValue,
    element::Root,
    platform::{Environment, Locale, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use std::time::Duration;

#[component]
fn Fixture() -> impl IntoElement {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
    let pane = signal(Pane {
        kind: PaneKind::Terminal,
        stream: "ime".into(),
        name: "IME".into(),
        controller: true,
        read_only: false,
        exit: None,
        unavailable: None,
        programs: Vec::new(),
        herdr_pane_id: None,
        herdr_grant: None,
        gap: false,
    });
    let ui = Ui::new(
        Model {
            phase: Phase::Ready,
            permission: Some(Permission::Operate),
            panes: vec![pane.get_untracked()],
            ..Model::default()
        },
        Client::fixture(sender),
    );
    ui.active.set("ime".into());
    let frames = signal(Vec::<String>::new());
    spawn(async move {
        while let Some(command) = receiver.recv().await {
            if let Command::Request(Operation::TerminalInput { stream, data }) = command {
                assert_eq!(stream, "ime");
                let data = String::from_utf8(decode(&data, 16 * 1024).unwrap()).unwrap();
                frames.update(|frames| frames.push(data));
            }
        }
    });
    ui.navigation.show_session();
    if !ui.navigation.composer_visible("ime") {
        ui.navigation.toggle_composer("ime");
    }
    let rendered = super::super::render(ui, pane.read_only());
    ui.refs
        .with(|refs| refs["ime"].write("keep中\x1b[?2004h".into()));
    let buffer = ui.refs.with(|refs| refs["ime"].buffer.clone());
    let saved_size = buffer.size.get_untracked();
    view()
        .flex_1()
        .child(rendered)
        .child(
            text(move || format!("frames={}", serde_json::to_string(&frames.get()).unwrap()))
                .h(px(20.0))
                .overflow_hidden(),
        )
        .child(text(move || {
            format!("source-size-preserved={}", buffer.size.get() == saved_size)
        }))
        .child(
            nsh_base::button()
                .on_press(move || pane.update(|pane| pane.read_only = true))
                .child(text("lose-control")),
        )
}

fn start(os: Os) -> Headless {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os,
            locale: Locale::parse("zh-CN").unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<Fixture>())
    })
    .unwrap();
    app.first_render().unwrap();
    app.press("id:terminal-pane-viewport").unwrap();
    app
}
fn input_id(app: &Headless) -> nsh_core::NodeId {
    app.screen()
        .layout_nodes()
        .iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-native-input"))
        .unwrap()
        .id
}
fn event(app: &mut Headless, kind: EventKind, payload: EventPayload) {
    app.engine()
        .control()
        .dispatch_event(input_id(app), kind, payload);
    app.wait(Duration::from_millis(20)).unwrap();
}
fn input_value(app: &Headless) -> String {
    app.screen()
        .input_text("terminal-native-input")
        .unwrap()
        .unwrap()
        .to_owned()
}
fn with_body(app: &Headless, body: &str) -> String {
    let value = input_value(app);
    let (_, old_body) = native_parts(&value);
    let header = &value[..value.len() - old_body.len()];
    format!("{header}{body}")
}
fn change(app: &mut Headless, body: &str) {
    let value = with_body(app, body);
    event(
        app,
        EventKind::ChangeText,
        EventPayload::Text {
            text: value,
            count: 1,
        },
    );
}
fn submit(app: &mut Headless, body: &str) {
    let value = with_body(app, body);
    event(
        app,
        EventKind::SubmitEditing,
        EventPayload::Text {
            text: value,
            count: 1,
        },
    );
}
fn composition(app: &mut Headless, active: bool, text: &str) {
    event(
        app,
        EventKind::CompositionChange,
        EventPayload::Map(vec![
            ("composing", PayloadValue::Bool(active)),
            ("text", PayloadValue::Str(text.into())),
        ]),
    );
}
fn frames(app: &Headless) -> Vec<String> {
    let screen = app.screen();
    let value = screen
        .texts()
        .into_iter()
        .find_map(|text| text.strip_prefix("frames="))
        .unwrap();
    serde_json::from_str(value).unwrap()
}

#[test]
fn real_terminal_view_buffers_native_candidate_events_commits_once_then_submits_enter() {
    for os in [Os::Linux, Os::MacOs, Os::Windows, Os::Android] {
        let mut app = start(os);
        composition(&mut app, true, "n");
        change(&mut app, "n");
        composition(&mut app, true, "ni");
        change(&mut app, "ni");
        assert!(
            frames(&app).is_empty(),
            "{os:?}: no candidates may enter the PTY"
        );
        change(&mut app, "你");
        assert!(frames(&app).is_empty());
        composition(&mut app, false, "");
        composition(&mut app, false, "");
        assert_eq!(frames(&app), ["你"]);
        submit(&mut app, "你");
        assert_eq!(frames(&app), ["你", "\r"]);
        app.press_key("ctrl+c").unwrap();
        assert_eq!(frames(&app), ["你", "\r", "\x03"]);
    }
}

#[test]
fn desktop_preedit_and_cancel_have_no_pty_effect_and_a_repeated_commit_is_not_lost() {
    for os in [Os::Linux, Os::Android] {
        let mut app = start(os);
        composition(&mut app, true, "ni");
        // GTK's preedit can remain outside the native value entirely.
        composition(&mut app, false, "");
        assert!(frames(&app).is_empty());
        composition(&mut app, true, "ni");
        change(&mut app, "ni");
        change(&mut app, "");
        composition(&mut app, false, "");
        assert!(frames(&app).is_empty());
        for (value, expected) in [("你", vec!["你"]), ("你你", vec!["你", "你"])] {
            composition(&mut app, true, "ni");
            change(&mut app, value);
            composition(&mut app, false, "");
            assert_eq!(frames(&app), expected);
        }
        composition(&mut app, true, "hao");
        change(&mut app, "你你hao");
        change(&mut app, "你你");
        composition(&mut app, false, "");
        assert_eq!(frames(&app), ["你", "你"]);
    }
}

#[test]
fn native_submit_keymap_modes_paste_and_composer_keep_their_own_input_routes() {
    let mut app = start(Os::Linux);
    app.type_text("terminal-native-input", "echo ").unwrap();
    app.press_key("enter").unwrap();
    app.press_key("ctrl+c").unwrap();
    app.press_key("up").unwrap();
    app.paste("terminal-native-input", &PasteEvent::new("中文\n下一行"))
        .unwrap();
    assert_eq!(
        frames(&app),
        [
            "e",
            "c",
            "h",
            "o",
            " ",
            "\r",
            "\x03",
            "\x1b[A",
            "\x1b[200~中文\r下一行\x1b[201~"
        ]
    );
    let before = frames(&app);
    app.type_text("会话输入", "composer草稿").unwrap();
    assert_eq!(frames(&app), before);
}

#[test]
fn losing_control_discards_the_pending_composition_and_rejects_queued_native_events() {
    let mut app = start(Os::Android);
    composition(&mut app, true, "ni");
    change(&mut app, "ni");
    app.press("lose-control").unwrap();
    change(&mut app, "你");
    composition(&mut app, false, "");
    submit(&mut app, "你");
    assert!(frames(&app).is_empty());
}

#[test]
fn software_backspace_and_submit_never_escape_an_active_composition() {
    let mut app = start(Os::Android);
    composition(&mut app, true, "ni");
    change(&mut app, "ni");
    event(
        &mut app,
        EventKind::KeyPress,
        EventPayload::Map(vec![("key", PayloadValue::Str("Backspace".into()))]),
    );
    change(&mut app, "n");
    submit(&mut app, "n");
    assert!(frames(&app).is_empty());
    change(&mut app, "你");
    composition(&mut app, false, "");
    change(&mut app, "你");
    assert_eq!(frames(&app), ["你"]);
    event(
        &mut app,
        EventKind::KeyPress,
        EventPayload::Map(vec![("key", PayloadValue::Str("Backspace".into()))]),
    );
    change(&mut app, "");
    assert_eq!(frames(&app), ["你", "\x7f"]);
    submit(&mut app, "");
    assert_eq!(frames(&app), ["你", "\x7f", "\r"]);
}

#[test]
fn rollover_keeps_queued_edits_and_distinguishes_a_full_repeated_commit() {
    let mut edit = Editing::default();
    let body = "猫".repeat(HISTORY_LIMIT);
    let old = native(&body);
    assert_eq!(edit.change(&old), body);
    let first_reset = edit.take_value().unwrap();
    assert!(first_reset.len() <= HEADER_BYTES + RETAINED_BYTES);

    // The count-protected native value may reject the first reset because this
    // edit was already made. It still belongs to the old input prefix.
    let queued = format!("{old}你");
    assert_eq!(edit.change(&queued), "你");
    let reset = edit.take_value().unwrap();
    assert!(native_parts(&reset).0.is_some());
    assert_ne!(
        first_reset, reset,
        "stale native resets must be retried as new props"
    );
    assert!(reset.len() <= HEADER_BYTES + RETAINED_BYTES);

    // After the native reset, even repeating the entire old value is new text.
    // A plain starts_with(old_text) heuristic would lose this commit.
    let repeated = format!("{reset}{body}你");
    assert_eq!(edit.change(&repeated), format!("{body}你"));
    let next_reset = edit.take_value().unwrap();
    assert!(native_parts(&next_reset).0.is_some());
    assert_ne!(next_reset, reset);
    assert!(next_reset.len() <= HEADER_BYTES + RETAINED_BYTES);
    assert_eq!(edit.change(&format!("{next_reset}🙂")), "🙂");
}

#[test]
fn oversized_composition_rolls_over_at_commit_then_handles_cancel_and_the_same_character() {
    let mut edit = Editing::default();
    let long = format!("{}e\u{301}🙂", "中".repeat(HISTORY_LIMIT));
    edit.composition(true);
    assert_eq!(edit.change(&native(&long)), "");
    assert!(edit.take_value().is_none(), "never reset an active preedit");
    assert_eq!(edit.composition(false), long);
    let mut value = edit.take_value().unwrap();
    assert!(value.len() <= HEADER_BYTES + RETAINED_BYTES);
    assert_eq!(edit.composition(false), "");

    for _ in 0..2 {
        edit.composition(true);
        assert_eq!(edit.change(&format!("{value}ni")), "");
        assert_eq!(edit.change(&value), "");
        assert_eq!(
            edit.composition(false),
            "",
            "a cancelled preedit is not text"
        );
        edit.composition(true);
        assert_eq!(edit.change(&format!("{value}你")), "");
        assert_eq!(edit.composition(false), "你");
        value.push('你');
    }
    assert_eq!(edit.submit(&value), "");
    let reset = edit.take_value().unwrap();
    assert_eq!(native_parts(&reset).1, "");
    assert!(native_parts(&reset).0.is_some());
    assert_eq!(edit.change(&format!("{reset}你")), "你");
}

#[test]
fn rejected_rollovers_keep_bounded_state_and_preserve_every_queued_append() {
    let mut edit = Editing::default();
    let chunk = "a".repeat(HISTORY_LIMIT);
    let mut native_value = Editing::initial_value();
    let mut previous_reset = String::new();
    for _ in 0..32 {
        native_value.push_str(&chunk);
        assert_eq!(edit.change(&native_value), chunk);
        let reset = edit.take_value().unwrap();
        assert!(reset.len() <= HEADER_BYTES + RETAINED_BYTES);
        assert_ne!(
            reset, previous_reset,
            "a rejected reset must not be deduplicated"
        );
        previous_reset = reset;
    }
    assert_eq!(edit.submit(&native_value), "");
    let reset = edit.take_value().unwrap();
    assert_eq!(native_parts(&reset).1, "");
    assert_eq!(edit.change(&format!("{reset}你")), "你");
}

#[test]
fn deleting_the_empty_sink_prefix_preserves_edits_queued_before_it_is_restored() {
    let mut edit = Editing::default();
    assert_eq!(edit.change(""), "");
    let reset = edit.take_value().unwrap();
    assert_eq!(edit.change("n"), "n");
    let newer_reset = edit.take_value().unwrap();
    assert_ne!(reset, newer_reset);
    assert_eq!(edit.change(&format!("{newer_reset}i")), "i");
    assert_eq!(edit.submit(&format!("{newer_reset}i")), "");
    assert_eq!(native_parts(&edit.take_value().unwrap()).1, "");
}

#[test]
fn native_long_input_stays_bounded_before_composition_cancel_repeated_commits_and_enter() {
    for os in [Os::Linux, Os::Android] {
        let mut app = start(os);
        let chunk = "a".repeat(16);
        for _ in 0..HISTORY_LIMIT * 2 / chunk.len() {
            app.type_text("terminal-native-input", &chunk).unwrap();
            assert!(input_value(&app).len() < HISTORY_LIMIT + HEADER_BYTES);
        }
        let typed = "a".repeat(HISTORY_LIMIT * 2);
        assert_eq!(frames(&app).concat(), typed);
        for committed in 0..2 {
            composition(&mut app, true, "ni");
            app.type_text("terminal-native-input", "ni").unwrap();
            assert_eq!(
                frames(&app).concat(),
                format!("{typed}{}", "你".repeat(committed))
            );
            // Use the native headless editor for edits/deletions so its text
            // and edit count stay in step with InputValue's stale-write guard.
            app.press_key("backspace").unwrap();
            app.press_key("backspace").unwrap();
            composition(&mut app, false, "");
            assert_eq!(
                frames(&app).concat(),
                format!("{typed}{}", "你".repeat(committed))
            );
            composition(&mut app, true, "ni");
            app.type_text("terminal-native-input", "你").unwrap();
            composition(&mut app, false, "");
        }
        assert_eq!(frames(&app).concat(), format!("{typed}你你"));
        app.press_key("enter").unwrap();
        assert_eq!(frames(&app).concat(), format!("{typed}你你\r"));
        assert!(native_parts(&input_value(&app)).0.is_some());
        assert_eq!(native_parts(&input_value(&app)).1, "");
        app.type_text("terminal-native-input", "你").unwrap();
        assert_eq!(frames(&app).concat(), format!("{typed}你你\r你"));
    }
}

#[test]
fn native_software_backspace_rearms_the_empty_sink_before_the_next_composition() {
    let mut app = start(Os::Android);
    app.clear("terminal-native-input").unwrap();
    assert_eq!(frames(&app), ["\x7f"]);
    assert!(native_parts(&input_value(&app)).0.is_some());
    assert_eq!(native_parts(&input_value(&app)).1, "");
    composition(&mut app, true, "ni");
    app.type_text("terminal-native-input", "你").unwrap();
    assert_eq!(frames(&app), ["\x7f"]);
    composition(&mut app, false, "");
    assert_eq!(frames(&app), ["\x7f", "你"]);
    app.press_key("enter").unwrap();
    assert_eq!(frames(&app), ["\x7f", "你", "\r"]);
}

fn output_touch(app: &mut Headless, phase: nsh_core::TouchPhase, time: f64, x: f32) {
    output_pointer(app, phase, time, x, nsh_core::PointerType::Touch);
}

fn output_pointer(
    app: &mut Headless,
    phase: nsh_core::TouchPhase,
    time: f64,
    x: f32,
    pointer_type: nsh_core::PointerType,
) {
    use nsh_core::{ROOT_ID, TouchEvent, TouchPhase, TouchPoint};
    let nodes = app.screen().layout_nodes();
    let card = nodes
        .iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-pane-viewport"))
        .unwrap();
    let row = nodes
        .iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-grid-row"))
        .unwrap();
    let touch = TouchEvent {
        phase,
        timestamp: time,
        activated: None,
        points: vec![TouchPoint {
            id: 17,
            target: row.id,
            path: if phase == TouchPhase::Start {
                vec![card.id]
            } else {
                Vec::new()
            },
            page: nsh_core::geometry::point(px(x), px(100.0)),
            location: nsh_core::geometry::point(px(x), px(5.0)),
            changed: true,
            pointer_type,
        }],
    };
    app.engine().control().dispatch_event(
        ROOT_ID,
        EventKind::Touch,
        EventPayload::Touch(Box::new(touch)),
    );
    app.wait(Duration::from_millis(20)).unwrap();
}

#[test]
fn tapping_selectable_child_without_a_parent_press_focuses_the_native_input() {
    use nsh_core::TouchPhase::{End, Start};
    for (os, pointer) in [
        (Os::Android, nsh_core::PointerType::Touch),
        (Os::Linux, nsh_core::PointerType::Mouse),
    ] {
        let mut app = start(os);
        app.type_text("会话输入", "草稿").unwrap();
        assert!(frames(&app).is_empty());
        output_pointer(&mut app, Start, 1000.0, 10.0, pointer);
        output_pointer(&mut app, End, 1080.0, 10.0, pointer);
        // Do not type_text(input): it focuses automatically and would hide the bug.
        app.press_key("x").unwrap();
        app.press_key("enter").unwrap();
        assert_eq!(frames(&app), ["x", "\r"], "{os:?}");
        assert_eq!(app.screen().input_text("会话输入").unwrap(), Some("草稿"));
        assert!(app.screen().matches("id:terminal-selection").unwrap());
    }
}

#[test]
fn selection_scroll_and_cancel_touches_do_not_steal_composer_focus() {
    use nsh_core::TouchPhase::{Cancel, End, Move, Start};
    for gesture in [
        vec![(Start, 0.0, 10.0), (End, 600.0, 10.0)],
        vec![(Start, 0.0, 10.0), (Move, 50.0, 30.0), (End, 100.0, 10.0)],
        vec![(Start, 0.0, 10.0), (Cancel, 50.0, 10.0)],
    ] {
        let mut app = start(Os::Android);
        app.type_text("会话输入", "草稿").unwrap();
        for (phase, time, x) in gesture {
            output_touch(&mut app, phase, time, x);
        }
        app.press_key("x").unwrap();
        assert!(frames(&app).is_empty());
        assert_eq!(app.screen().input_text("会话输入").unwrap(), Some("草稿x"));
        assert!(app.screen().matches("id:terminal-selection").unwrap());
    }
}

#[test]
fn blurred_input_resets_are_distinct_and_reject_the_old_native_generation() {
    let mut edit = Editing::default();
    assert_eq!(edit.change(&native("old")), "old");
    let reset = edit.reset();
    let next = edit.reset();
    assert_ne!(reset, next);
    assert_eq!(edit.change(&native("oldlate")), "");
    assert_eq!(edit.change(&format!("{reset}late")), "");
    assert_eq!(edit.change(&format!("{next}你")), "你");
}

#[test]
fn backspacing_part_of_the_hidden_header_never_exports_metadata() {
    let mut edit = Editing::default();
    let mut value = Editing::initial_value();
    value.pop();
    assert_eq!(edit.change(&value), "");
    let reset = edit.take_value().unwrap();
    assert_eq!(native_parts(&reset).1, "");
    assert_eq!(edit.change(&format!("{reset}你")), "你");
}

fn end_selection_requests(app: &Headless) -> usize {
    let id = input_id(app);
    app.screen().commands().iter().filter(|(target, command)| {
        *target == id && matches!(command, nsh_core::Command::SetSelection { start, end } if *start == NATIVE_END && *end == NATIVE_END)
    }).count()
}

#[test]
fn initial_rollover_and_equal_length_resets_request_the_native_current_end() {
    let mut app = start(Os::Android);
    assert!(
        end_selection_requests(&app) > 0,
        "initial header needs an explicit caret"
    );
    for _ in 0..2 {
        let before = end_selection_requests(&app);
        app.press_key("enter").unwrap();
        assert_eq!(
            end_selection_requests(&app),
            before + 1,
            "equal-length headers still need a caret command"
        );
        assert_eq!(native_parts(&input_value(&app)).1, "");
    }
    let before = end_selection_requests(&app);
    let long = "中🙂".repeat(HISTORY_LIMIT / 7 + 1);
    change(&mut app, &long);
    assert_eq!(end_selection_requests(&app), before + 1);
    assert!(native_parts(&input_value(&app)).1.len() <= RETAINED_BYTES);

    let before = end_selection_requests(&app);
    composition(&mut app, true, "ni");
    app.type_text("terminal-native-input", "ni").unwrap();
    assert_eq!(
        end_selection_requests(&app),
        before,
        "never reposition an active preedit"
    );
    app.press_key("backspace").unwrap();
    app.press_key("backspace").unwrap();
    composition(&mut app, false, "");
    assert_eq!(end_selection_requests(&app), before);
}

#[test]
fn hardware_enter_is_consumed_before_native_editors_and_leaves_ime_actions_separate() {
    for os in [Os::Android, Os::Linux, Os::Windows, Os::MacOs] {
        let mut app = start(os);
        let (_, bytes) = keys::bindings(matches!(os, Os::MacOs))
            .into_iter()
            .find(|(key, _)| *key == "enter")
            .expect("hardware Enter must not depend on the native editor submitting");
        assert_eq!(bytes.encode(Default::default()), "\r");
        app.press_key("enter").unwrap();
        assert_eq!(frames(&app), ["\r"]);
        composition(&mut app, true, "ni");
        change(&mut app, "你");
        composition(&mut app, false, "");
        submit(&mut app, "你");
        assert_eq!(frames(&app), ["\r", "你", "\r"]);
    }
}

#[test]
fn focusing_a_mounted_native_editor_restores_its_caret_after_initial_prop_ordering() {
    let mut app = start(Os::Android);
    let before = end_selection_requests(&app);
    event(&mut app, EventKind::Focus, EventPayload::None);
    assert_eq!(end_selection_requests(&app), before + 1);
    assert!(frames(&app).is_empty());
}
