use super::*;

fn shell(keys: &str) -> ClientShellState {
    let config: Config = toml::from_str(&format!("[keys]\n{keys}")).expect("valid keys");
    assert!(config.collect_diagnostics().is_empty());
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state
}

#[test]
fn layout_shortcuts_enter_prefix_with_kitty_metadata_or_russian_fallback() {
    for input in [
        "\x1b[1092::97;3u", // Russian ф on A.
        "\x1bф",
        "\x1b[945::97;3u", // Greek α on A.
    ] {
        let mut state = shell("prefix = 'alt+a'");
        let result = state.handle_input_bytes(input.as_bytes());
        assert_eq!(state.mode, ClientShellMode::Prefix, "{input:?}");
        assert!(result.requests.is_empty());
        let release = state.handle_input_bytes(b"\x1b[1092::97;1:3u");
        assert!(release.requests.is_empty(), "consumed prefix release");
    }
}

#[test]
fn layout_shortcuts_resolve_prefix_and_direct_actions() {
    for input in ["\x1b[1081::113u", "й"] {
        let mut state = shell("");
        state.handle_input_bytes(b"\x02");
        assert!(
            state.handle_input_bytes(input.as_bytes()).detach,
            "{input:?}"
        );
    }
    let mut state = shell("detach = 'alt+q'");
    assert!(state.handle_input_bytes(b"\x1b[1081::113;3u").detach);
}

#[test]
fn layout_shortcuts_preserve_exact_character_binding_precedence() {
    let mut state = shell("help = 'prefix+с'");
    state.handle_input_bytes(b"\x02");
    state.handle_input_bytes(b"\x1b[1089::99u");
    assert!(matches!(state.overlay, Some(ClientShellOverlay::Help(_))));
}

#[test]
fn layout_shortcuts_match_shifted_letters_and_physical_punctuation() {
    for input in ["\x1b[1099:1067:115;2u", "Ы"] {
        let mut state = shell("settings = 'prefix+S'");
        state.handle_input_bytes(b"\x02");
        state.handle_input_bytes(input.as_bytes());
        assert!(
            matches!(state.overlay, Some(ClientShellOverlay::Settings(_))),
            "{input:?}"
        );
    }
    let mut state = shell("split_vertical = 'prefix+|'");
    state.handle_input_bytes(b"\x02");
    let result = state.handle_input_bytes(b"\x1b[92:47;2u");
    assert!(result.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(request.method, crate::api::schema::Method::PaneSplit(_))
    )));
}

#[test]
fn layout_shortcuts_do_not_translate_unconsumed_pane_keys_or_rename_text() {
    let mut state = shell("");
    let result = state.handle_input_bytes(b"\x1b[1092::97;3u");
    assert!(matches!(
        &result.requests[..],
        [ClientMessage::ClientShellPaneInput { events, .. }]
            if matches!(&events[..], [ClientPaneInputEvent::Key {
                code: crate::protocol::ClientKeyCode::Char('ф'), modifiers, ..
            }] if *modifiers == KeyModifiers::ALT.bits())
    ));
    state.open_rename_pane_overlay();
    state.handle_input_bytes(b"\x1b[1089::99;1;1089u");
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::Rename(rename)) if rename.input.as_str() == "с"
    ));
}

#[test]
fn layout_shortcuts_navigate_resize_and_command_overlays() {
    let mut state = shell("");
    state.handle_input_bytes(b"\x02");
    state.handle_input_bytes("ц".as_bytes());
    assert_eq!(state.mode, ClientShellMode::Navigate);

    let mut state = shell("");
    state.handle_input_bytes(b"\x02");
    state.handle_input_bytes("к".as_bytes());
    assert_eq!(state.mode, ClientShellMode::Resize);
    let result = state.handle_input_bytes("д".as_bytes());
    assert!(result.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(request.method, crate::api::schema::Method::PaneResize(_))
    )));

    let mut state = shell("");
    state.overlay = Some(ClientShellOverlay::Help(ClientHelpOverlay {
        query: TextEditor::default(),
        search_focused: false,
        scroll: 0,
    }));
    state.handle_input_bytes(b"\x1b[46:44:47;2u");
    assert!(state.overlay.is_none());
}

#[test]
fn layout_shortcuts_exact_prefix_and_commands_win_over_aliases() {
    let mut state = shell("prefix = 'alt+ф'\ndetach = 'alt+a'");
    let result = state.handle_input_bytes(b"\x1b[1092::97;3u");
    assert!(!result.detach);
    assert_eq!(state.mode, ClientShellMode::Prefix);

    let mut state = shell("prefix = 'a'\nhelp = 'prefix+ф'");
    state.handle_input_bytes(b"a");
    state.handle_input_bytes(b"\x1b[1092::97u");
    assert!(matches!(state.overlay, Some(ClientShellOverlay::Help(_))));

    let mut state = shell("help = 'alt+a'\ndetach = 'alt+ф'");
    assert!(state.handle_input_bytes(b"\x1b[1092::97;3u").detach);
}

#[test]
fn layout_shortcuts_keep_altgr_text_out_of_direct_aliases() {
    let mut state = shell("detach = 'ctrl+alt+a'");
    let result = state.handle_input_bytes(b"\x1b[1092::97;7;1092u");
    assert!(!result.detach);
    assert!(matches!(
        &result.requests[..],
        [ClientMessage::ClientShellPaneInput { events, .. }]
            if matches!(&events[..], [ClientPaneInputEvent::Key {
                generated_text: Some(text), ..
            }] if text == "ф")
    ));
}

#[test]
fn layout_shortcuts_navigation_prefers_custom_character_bindings() {
    for (keys, expected) in [("", "ws_2"), ("navigate_workspace_up = 'о'", "ws_3")] {
        let mut state = shell(&format!("navigate_workspace_down = 'j'\n{keys}"));
        let mut projection = snapshot();
        let mut second = projection.workspaces[0].clone();
        second.workspace_id = "ws_2".into();
        second.number = 2;
        second.label = "second".into();
        second.focused = false;
        let mut third = second.clone();
        third.workspace_id = "ws_3".into();
        third.number = 3;
        third.label = "third".into();
        projection.workspaces.extend([second, third]);
        state.set_snapshot(Box::new(projection));
        state.compose(120, 40).expect("layout");
        state.handle_input_bytes(b"\x02w");
        state.handle_input_bytes(b"\x1b[1086::106u");
        assert_eq!(
            state
                .navigate_workspace_id
                .as_ref()
                .expect("navigation target")
                .workspace_id,
            expected
        );
    }
}

#[test]
fn layout_shortcuts_shifted_g_selects_the_last_navigator_pane() {
    let mut state = shell("");
    state.config.navigator_start_expanded = true;
    state.config.navigator_start_search_focused = false;
    let mut projection = snapshot();
    for index in 2..=3 {
        let mut pane = projection.panes[0].clone();
        pane.pane_id = format!("pane_{index}");
        pane.focused = false;
        projection.panes.push(pane);
    }
    state.set_snapshot(Box::new(projection));
    state.open_navigator_overlay();
    state.handle_input_bytes(b"\x1b[1087:1055:103;2u");
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(ClientNavigatorOverlay {
            selected: Some(ClientNavigatorTarget::Pane { ref pane_id, .. }), ..
        })) if pane_id == "pane_3"
    ));
}

#[test]
fn layout_shortcuts_ctrl_bracket_stays_distinct_from_ctrl_shift_bracket() {
    let ctrl = crate::input::parse_terminal_key_sequence("\x1b[1093::91;5u").expect("Ctrl+[");
    let shifted =
        crate::input::parse_terminal_key_sequence("\x1b[1093:1061:91;6u").expect("Ctrl+Shift+[");
    assert!(input::is_ctrl_bracket_key(&ctrl));
    assert!(!input::is_ctrl_bracket_key(&shifted));
}

#[test]
fn layout_shortcuts_copy_motion_and_search_keep_command_and_text_separate() {
    let mut state = shell("");
    let mut frame = surface();
    frame.panes[0].scroll = Some(crate::protocol::PaneSurfaceScrollMetrics {
        offset_from_bottom: 0,
        max_offset_from_bottom: 10,
        viewport_rows: 2,
    });
    state.set_pane_surface(frame);
    state.compose(106, 20).expect("layout");
    state.handle_input_bytes(b"\x02");
    state.handle_input_bytes("х".as_bytes());
    assert_eq!(state.mode, ClientShellMode::Copy);
    let row = state.copy_mode.as_ref().expect("copy mode").cursor.row;
    state.handle_input_bytes("л".as_bytes());
    assert_eq!(
        state.copy_mode.as_ref().expect("copy mode").cursor.row,
        row - 1
    );
    state.handle_input_bytes(b"\x1b[46::47u");
    state.handle_input_bytes("йцукен".as_bytes());
    assert_eq!(
        state
            .copy_mode
            .as_ref()
            .expect("copy mode")
            .search_prompt
            .as_ref()
            .expect("search")
            .query
            .as_str(),
        "йцукен"
    );
}
