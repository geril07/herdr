use super::*;

#[test]
fn sidebar_position_places_chrome_on_the_configured_edge() {
    for (position, sidebar_x, pane_x, divider_x, toggle_glyph) in [
        (crate::config::SidebarPositionConfig::Left, 0, 26, 25, "«"),
        (crate::config::SidebarPositionConfig::Right, 80, 0, 80, "»"),
    ] {
        let mut base = Config::default();
        base.ui.sidebar_position = position;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&base));
        state.set_snapshot(Box::new(snapshot()));
        state.set_pane_surface(surface());
        let frame = state.compose(106, 20).expect("composed frame");
        let layout = state.layout(106, 20);
        assert_eq!((layout.sidebar.x, layout.sidebar.width), (sidebar_x, 26));
        assert_eq!(
            (layout.pane_surface.x, layout.pane_surface.width),
            (pane_x, 80)
        );
        assert_eq!(state.hits.sidebar_divider.x, divider_x);
        let toggle = state.hits.sidebar_toggle;
        let toggle_cell =
            &frame.cells[usize::from(toggle.y) * usize::from(frame.width) + usize::from(toggle.x)];
        assert_eq!(toggle_cell.symbol.as_str(), toggle_glyph);
    }
}

#[test]
fn tab_overflow_controls_scroll_the_client_owned_tab_bar() {
    let mut snapshot = snapshot();
    snapshot.tabs.extend((2..=8).map(|number| ClientShellTab {
        tab_id: format!("tab_{number}"),
        workspace_id: "ws_1".into(),
        number,
        label: number.to_string(),
        custom_label: false,
        zoomed: false,
        focused: false,
        agent_status: AgentStatus::Idle,
    }));
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(80, 20).expect("overflow tab bar");

    assert!(state.hits.tab_scroll_right.width > 0);
    let scroll_right = state.hits.tab_scroll_right;
    let outcome =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: scroll_right.x + 1,
            row: scroll_right.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(outcome.repaint);
    assert_eq!(state.tab_scroll, 1);

    let mut update = state.snapshot.as_deref().expect("snapshot").clone();
    update.focused_tab_id = Some("tab_8".into());
    for tab in &mut update.tabs {
        tab.focused = tab.tab_id == "tab_8";
    }
    state.set_snapshot(Box::new(update));
    state.compose(80, 20).expect("focused overflow tab");
    assert!(state.hits.tabs.iter().any(|(_, tab_id)| tab_id == "tab_8"));

    state.compose(300, 20).expect("tabs without overflow");
    assert_eq!(state.tab_scroll, 0);
    assert_eq!(state.hits.tabs.len(), 8);
    state.compose(80, 20).expect("focused tab after narrowing");
    assert!(state.hits.tabs.iter().any(|(_, tab_id)| tab_id == "tab_8"));
}

#[test]
fn focused_last_overflow_tab_shows_its_full_label() {
    let mut projected = snapshot();
    let labels = [
        "1",
        "Laiza Portfolio Site",
        "linkedin posts",
        "update cv",
        "nvim",
        "brother",
        "day organiser",
        "nvim test",
    ];
    projected.tabs = labels
        .iter()
        .enumerate()
        .map(|(index, label)| ClientShellTab {
            tab_id: format!("tab_{}", index + 1),
            workspace_id: "ws_1".into(),
            number: index + 1,
            label: (*label).into(),
            custom_label: index > 0,
            zoomed: false,
            focused: index == 7,
            agent_status: AgentStatus::Idle,
        })
        .collect();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    for number in [8, 7, 8] {
        let tab_id = format!("tab_{number}");
        projected.focused_tab_id = Some(tab_id.clone());
        projected.workspaces[0].active_tab_id = tab_id.clone();
        projected.panes[0].tab_id = tab_id.clone();
        for tab in &mut projected.tabs {
            tab.focused = tab.tab_id == tab_id;
        }
        state.set_snapshot(Box::new(projected.clone()));
        state.set_pane_surface(surface());
        let frame = state
            .compose(133, 20)
            .expect("reporter's overflowing strip");
        assert_eq!(
            state.hits.new_tab.right() - state.hits.tab_scroll_left.x,
            107
        );
        let rect = state
            .hits
            .tabs
            .iter()
            .find(|(_, id)| id == &tab_id)
            .expect("focused tab")
            .0;
        let text = (rect.x..rect.right())
            .map(|x| {
                frame.cells[(rect.y * frame.width + x) as usize]
                    .symbol
                    .as_str()
            })
            .collect::<String>();
        assert!(
            text.contains(labels[number - 1]),
            "focused tab rendered as {text:?}, rect={rect:?}"
        );
    }

    // Manual scrolling must be able to reveal the rest of a partially drawn last tab.
    let end_scroll = state.tab_scroll;
    let left = state.hits.tab_scroll_left;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: left.x + 1,
        row: left.y,
        modifiers: KeyModifiers::empty(),
    })]);
    let frame = state.compose(133, 20).expect("manual scroll left");
    assert_eq!(state.tab_scroll, end_scroll - 1);
    let right = state.hits.tab_scroll_right;
    assert_eq!(
        frame.cells[(right.y * frame.width + right.x + 1) as usize].fg,
        crate::protocol::color_to_u32(state.config.palette.overlay1),
        "right arrow stays enabled while the final tab is clipped"
    );
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: right.x + 1,
        row: right.y,
        modifiers: KeyModifiers::empty(),
    })]);
    let frame = state.compose(133, 20).expect("manual scroll right");
    assert_eq!(state.tab_scroll, end_scroll);
    assert!(frame_rows(&frame)[0].contains("nvim test"));
    assert_eq!(
        frame.cells[(right.y * frame.width + right.x + 1) as usize].fg,
        crate::protocol::color_to_u32(state.config.palette.overlay0),
        "right arrow dims at the useful scroll limit"
    );
}

#[test]
fn focused_workspace_change_reveals_new_workspace_in_full_sidebar() {
    let mut initial = snapshot();
    let template = initial.workspaces[0].clone();
    initial.workspaces = (1..=12)
        .map(|number| ClientShellWorkspace {
            workspace_id: format!("ws_{number}"),
            number,
            label: format!("space-{number}"),
            branch: None,
            focused: number == 1,
            ..template.clone()
        })
        .collect();

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(initial));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("full sidebar");
    assert!(state.hits.workspace_max_scroll > 0);
    assert!(state
        .hits
        .workspaces
        .iter()
        .all(|hit| hit.workspace_id != "ws_12"));

    let mut update = state.snapshot.as_deref().expect("snapshot").clone();
    update.revision = 2;
    update.focused_workspace_id = Some("ws_12".into());
    for workspace in &mut update.workspaces {
        workspace.focused = workspace.workspace_id == "ws_12";
    }
    let mut updated_surface = surface();
    updated_surface.projection_revision = 2;
    state.set_snapshot(Box::new(update));
    state.set_pane_surface(updated_surface);
    state.compose(106, 2).expect("zero-height workspace body");
    assert!(state.reveal_focused_workspace);
    state.compose(106, 20).expect("updated full sidebar");

    assert!(state
        .hits
        .workspaces
        .iter()
        .any(|hit| hit.workspace_id == "ws_12"));
}

#[test]
fn client_owned_sidebar_dividers_resize_live() {
    for position in [
        crate::config::SidebarPositionConfig::Left,
        crate::config::SidebarPositionConfig::Right,
    ] {
        // Dragging the width divider toward the pane surface grows the sidebar
        // on either side: rightward for a left sidebar, leftward for a right one.
        let (drag_first, divider_first, drag_second, divider_second) = match position {
            crate::config::SidebarPositionConfig::Left => (31, 31, 32, 32),
            crate::config::SidebarPositionConfig::Right => (74, 74, 73, 73),
        };
        let mut base = Config::default();
        base.ui.sidebar_position = position;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&base));
        state.set_snapshot(Box::new(snapshot()));
        state.set_pane_surface(surface());
        state.compose(106, 30).expect("expanded sidebar");
        assert!(state.hits.machines.is_empty());
        let workspace_body = state.hits.workspace_body;
        let needless_scroll =
            state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: workspace_body.x,
                row: workspace_body.y,
                modifiers: KeyModifiers::empty(),
            })]);
        assert_eq!(state.hits.workspace_max_scroll, 0);
        assert_eq!(state.workspace_scroll, 0);
        assert!(!needless_scroll.repaint);
        let width_divider = state.hits.sidebar_divider;
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: width_divider.x,
            row: width_divider.y + 2,
            modifiers: KeyModifiers::empty(),
        })]);
        let resize =
            state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: drag_first,
                row: width_divider.y + 2,
                modifiers: KeyModifiers::empty(),
            })]);
        assert_eq!(state.sidebar_width, 32);
        assert!(state.sidebar_width_manual);
        assert!(resize.repaint);
        assert!(resize.resize);
        let waiting_frame = state.compose(106, 30).expect("waiting for resized surface");
        let waiting_text: String = waiting_frame
            .cells
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect();
        assert!(
            waiting_text.contains(" spaces"),
            "local sidebar must keep spaces while resizing: {waiting_text}"
        );
        assert!(!waiting_text.contains(" machines"));
        assert!(!waiting_text.contains("Select a connected machine"));
        assert!(!waiting_text.contains("LIVE"));
        assert!(waiting_frame.cursor.is_none());
        assert!(state.pane_surface.is_none());
        assert!(state.hits.panes.is_empty());
        assert!(state.hits.pane_splits.is_empty());
        assert!(state.hits.machines.is_empty());
        assert_eq!(state.hits.sidebar_divider.x, divider_first);
        assert_eq!(state.hits.workspaces[0].workspace_id, "ws_1");

        let next_resize =
            state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: drag_second,
                row: width_divider.y + 2,
                modifiers: KeyModifiers::empty(),
            })]);
        assert!(next_resize.resize);
        state.compose(106, 30).expect("continued resize");
        assert_eq!(state.hits.sidebar_divider.x, divider_second);
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: drag_second,
            row: width_divider.y + 2,
            modifiers: KeyModifiers::empty(),
        })]);
        assert!(state.chrome_drag.is_none());

        state.set_pane_surface(surface());
        let recovered_frame = state.compose(106, 30).expect("resized sidebar");
        let recovered_text: String = recovered_frame
            .cells
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect();
        assert!(recovered_text.contains(" spaces"));
        assert!(recovered_text.contains("LIVE"));
        assert!(!state.hits.panes.is_empty());
        let section_divider = state.hits.sidebar_section_divider;
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: section_divider.x + 2,
            row: section_divider.y,
            modifiers: KeyModifiers::empty(),
        })]);
        let split =
            state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: section_divider.x + 2,
                row: 20,
                modifiers: KeyModifiers::empty(),
            })]);
        assert!(state.sidebar_section_split > 0.6);
        assert!(split.repaint);
        assert!(!split.resize);
    }
}

#[test]
fn context_menus_capture_stable_targets_and_route_actions() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("composed frame");

    let workspace = state.hits.workspaces[0].rect;
    let open_workspace_menu =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: workspace.x + 2,
            row: workspace.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(open_workspace_menu.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Workspace { ref workspace_id, .. },
            ..
        })) if workspace_id == "ws_1"
    ));
    let workspace_items = match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu.items(),
        _ => panic!("workspace context menu"),
    };
    assert!(workspace_items
        .iter()
        .any(|item| item.action == ClientContextMenuAction::NewWorktree));
    state.compose(106, 20).expect("workspace context menu");
    let rename = state.hits.context_menu_rows[0].0;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rename.x + 1,
        row: rename.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            target: ClientRenameTarget::Workspace { ref workspace_id },
            ..
        })) if workspace_id == "ws_1"
    ));

    state.overlay = None;
    state.compose(106, 20).expect("composed frame");
    let pane = state.hits.panes[0].rect;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: pane.x + 1,
        row: pane.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.compose(106, 20).expect("pane context menu");
    let split_index = match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .position(|item| item.action == ClientContextMenuAction::SplitRight)
            .expect("split right item"),
        _ => panic!("pane context menu"),
    };
    let split = state.hits.context_menu_rows[split_index].0;
    let outcome =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: split.x + 1,
            row: split.y,
            modifiers: KeyModifiers::empty(),
        })]);
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("pane split context action should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneSplit(params)
            if params.target_pane_id.as_deref() == Some("pane_1")
                && params.direction == crate::api::schema::SplitDirection::Right
    ));
}

#[test]
fn global_menu_opens_from_sidebar_and_routes_client_actions() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("shell frame");
    let launcher = state.hits.global_launcher;
    assert_ne!(launcher, Rect::default());

    let open = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: launcher.x,
        row: launcher.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(open.repaint);
    let menu = state.compose(106, 30).expect("global menu");
    let text = menu
        .cells
        .chunks(menu.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("settings"));
    assert!(text.contains("keybinds"));
    assert!(text.contains("reload config"));
    assert!(text.contains("detach"));

    let keybinds = state.hits.global_menu_rows[1].0;
    let help = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: keybinds.x,
        row: keybinds.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(help.actions.is_empty());
    assert!(matches!(state.overlay, Some(ClientShellOverlay::Help(_))));

    state.overlay = Some(ClientShellOverlay::GlobalMenu(ClientGlobalMenuOverlay {
        highlighted: 3,
    }));
    let detach = state.handle_input_bytes(b"\r");
    assert!(detach.detach);
    assert!(state.overlay.is_none());
}

#[test]
fn new_tab_overlay_owns_text_cursor_and_submits_public_api_request() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut open = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewTab),
        &mut open,
    );
    assert!(open.actions.is_empty());
    let frame = state.compose(106, 20).expect("new tab overlay");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("new tab"));
    assert!(text.contains("save"));
    let restored = frame.to_ratatui_buffer().expect("overlay frame");
    assert!(!restored
        .cell((26, 7))
        .expect("overlay title cell")
        .modifier
        .contains(Modifier::DIM));
    assert!(frame.cursor.as_ref().is_some_and(|cursor| cursor.visible));

    assert!(state.handle_input_bytes(b"logs").actions.is_empty());
    let create = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &create.actions[..] else {
        panic!("new tab save should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::TabCreate(params)
            if params.workspace_id.as_deref() == Some("ws_1")
                && params.label.as_deref() == Some("logs")
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn close_confirmation_error_becomes_client_owned_overlay_and_stable_group_close() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_pane_close = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
        &mut close,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &close.actions[..] else {
        panic!("pane close should use endpoint API");
    };
    let request_id = request.id.clone();
    assert!(
        state
            .handle_endpoint_result(
                "boot-1",
                &request_id,
                Err(ClientShellEndpointError {
                    code: Some("confirmation_required".into()),
                    message: "confirmation required".into(),
                }),
            )
            .0
    );
    let frame = state.compose(106, 20).expect("confirmation overlay");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Close workspace?"));
    assert!(text.contains("1 pane"));

    let confirm = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
        panic!("confirmation should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceClose(params)
            if params.workspace_id == "ws_1" && params.close_group
    ));
}

#[test]
fn pane_close_keybind_confirms_before_closing_when_enabled() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    assert!(state.config.confirm_pane_close);
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
        &mut close,
    );
    assert!(close.actions.is_empty());
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::ConfirmClose(ClientConfirmCloseOverlay {
            target: ClientConfirmCloseTarget::Pane { pane_id, .. },
            ..
        })) if pane_id == "pane_1"
    ));
    let frame = state.compose(106, 20).expect("pane confirmation overlay");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Close pane?"));
    assert!(text.contains("pane_1"));

    // Focus may move before confirming; the captured pane id still closes.
    state.snapshot.as_mut().expect("snapshot").focused_pane_id = Some("pane_2".into());
    let confirm = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
        panic!("pane confirmation should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_1"
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn pane_close_keybind_cancel_keeps_pane() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
        &mut close,
    );
    assert!(close.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
    let cancel = state.handle_input_bytes(b"\x1b");
    assert!(cancel.actions.is_empty());
    assert!(state.overlay.is_none());
}

#[test]
fn confirm_accept_alias_confirms_close_dialog() {
    let config: Config = toml::from_str("[keys]\nconfirm_accept = \"y\"\n").unwrap();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
        &mut close,
    );
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
    let frame = state.compose(106, 20).expect("pane confirmation overlay");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("/y confirm"),
        "alias should be visible on the accept button"
    );
    let confirm = state.handle_input_bytes(b"y");
    let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
        panic!("pane confirmation alias should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_1"
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn confirm_dialog_ignores_y_without_alias() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
        &mut close,
    );
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
    let ignored = state.handle_input_bytes(b"y");
    assert!(ignored.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
}

#[test]
fn pane_close_keybind_closes_directly_when_disabled() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_pane_close = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
        &mut close,
    );
    assert!(state.overlay.is_none());
    let [ClientShellAction::Endpoint { request, .. }] = &close.actions[..] else {
        panic!("pane close should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_1"
    ));
}

#[test]
fn pane_context_menu_close_follows_pane_confirmation_setting() {
    for confirm_pane_close in [true, false] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.config.confirm_pane_close = confirm_pane_close;
        state.set_snapshot(Box::new(snapshot()));
        state.set_pane_surface(surface());
        state.compose(106, 20).expect("composed frame");
        let pane = state.hits.panes[0].rect;
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: pane.x + 1,
            row: pane.y,
            modifiers: KeyModifiers::empty(),
        })]);
        state.compose(106, 20).expect("pane context menu");
        let close_index = match state.overlay.as_ref() {
            Some(ClientShellOverlay::ContextMenu(menu)) => menu
                .items()
                .iter()
                .position(|item| item.action == ClientContextMenuAction::ClosePane)
                .expect("close pane item"),
            _ => panic!("pane context menu"),
        };
        let close = state.hits.context_menu_rows[close_index].0;
        let outcome =
            state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: close.x + 1,
                row: close.y,
                modifiers: KeyModifiers::empty(),
            })]);
        if confirm_pane_close {
            assert!(outcome.actions.is_empty());
            assert!(matches!(
                state.overlay.as_ref(),
                Some(ClientShellOverlay::ConfirmClose(ClientConfirmCloseOverlay {
                    target: ClientConfirmCloseTarget::Pane { pane_id, .. },
                    ..
                })) if pane_id == "pane_1"
            ));
            let confirm = state.handle_input_bytes(b"\r");
            let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
                panic!("pane confirmation should use endpoint API");
            };
            assert!(matches!(
                &request.method,
                crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_1"
            ));
        } else {
            assert!(state.overlay.is_none());
            let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
                panic!("pane close should use endpoint API");
            };
            assert!(matches!(
                &request.method,
                crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_1"
            ));
        }
    }
}

#[test]
fn tab_close_keybind_confirms_before_closing_when_enabled() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    assert!(state.config.confirm_tab_close);
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::CloseTab),
        &mut close,
    );
    assert!(close.actions.is_empty());
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::ConfirmClose(ClientConfirmCloseOverlay {
            target: ClientConfirmCloseTarget::Tab { tab_id, .. },
            ..
        })) if tab_id == "tab_1"
    ));
    let frame = state.compose(106, 20).expect("tab confirmation overlay");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Close tab?"));
    assert!(text.contains('1'));

    // Focus may move before confirming; the captured tab id still closes.
    state.snapshot.as_mut().expect("snapshot").focused_tab_id = Some("tab_2".into());
    let confirm = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
        panic!("tab confirmation should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::TabClose(params) if params.tab_id == "tab_1"
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn tab_close_keybind_cancel_keeps_tab() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::CloseTab),
        &mut close,
    );
    assert!(close.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
    let cancel = state.handle_input_bytes(b"\x1b");
    assert!(cancel.actions.is_empty());
    assert!(state.overlay.is_none());
}

#[test]
fn tab_close_keybind_closes_directly_when_disabled() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_tab_close = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::CloseTab),
        &mut close,
    );
    assert!(state.overlay.is_none());
    let [ClientShellAction::Endpoint { request, .. }] = &close.actions[..] else {
        panic!("tab close should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::TabClose(params) if params.tab_id == "tab_1"
    ));
}

#[test]
fn tab_context_menu_close_follows_tab_confirmation_setting() {
    for confirm_tab_close in [true, false] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.config.confirm_tab_close = confirm_tab_close;
        state.set_snapshot(Box::new(snapshot()));
        state.set_pane_surface(surface());
        state.open_tab_context_menu("tab_1".into(), 0, 0);
        let close_index = match state.overlay.as_ref() {
            Some(ClientShellOverlay::ContextMenu(menu)) => menu
                .items()
                .iter()
                .position(|item| item.action == ClientContextMenuAction::Close)
                .expect("close tab item"),
            _ => panic!("tab context menu"),
        };
        let mut outcome = ClientShellInput::default();
        state.activate_context_menu_item(close_index, &mut outcome);
        if confirm_tab_close {
            assert!(matches!(
                state.overlay.as_ref(),
                Some(ClientShellOverlay::ConfirmClose(ClientConfirmCloseOverlay {
                    target: ClientConfirmCloseTarget::Tab { tab_id, .. },
                    ..
                })) if tab_id == "tab_1"
            ));
            let confirm = state.handle_input_bytes(b"\r");
            let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
                panic!("tab confirmation should use endpoint API");
            };
            assert!(matches!(
                &request.method,
                crate::api::schema::Method::TabClose(params) if params.tab_id == "tab_1"
            ));
        } else {
            assert!(state.overlay.is_none());
        }
        let [.., ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
            panic!("tab context action should use endpoint API");
        };
        if confirm_tab_close {
            // Confirming path only focuses; the close itself comes from the overlay.
            assert!(matches!(
                &request.method,
                crate::api::schema::Method::TabFocus(params) if params.tab_id == "tab_1"
            ));
        } else {
            assert!(matches!(
                &request.method,
                crate::api::schema::Method::TabClose(params) if params.tab_id == "tab_1"
            ));
        }
    }
}
