use super::*;

fn search_snapshot(count: usize) -> ClientShellSnapshot {
    let mut projected = snapshot();
    let workspace = projected.workspaces[0].clone();
    let tab = projected.tabs[0].clone();
    let pane = projected.panes[0].clone();
    projected.workspaces.clear();
    projected.tabs.clear();
    projected.panes.clear();
    for index in 1..=count {
        let workspace_id = format!("ws_{index}");
        let tab_id = format!("tab_{index}");
        let pane_id = format!("pane_{index}");
        projected.workspaces.push(ClientShellWorkspace {
            workspace_id: workspace_id.clone(),
            active_tab_id: tab_id.clone(),
            label: format!("project {index}"),
            number: index,
            focused: index == 1,
            branch: Some(format!("feature/task-{index}")),
            ..workspace.clone()
        });
        projected.tabs.push(ClientShellTab {
            workspace_id: workspace_id.clone(),
            tab_id: tab_id.clone(),
            label: format!("task {index}"),
            focused: index == 1,
            ..tab.clone()
        });
        projected.panes.push(ClientShellPane {
            pane_id: pane_id.clone(),
            workspace_id: workspace_id.clone(),
            tab_id: tab_id.clone(),
            focused: index == 1,
            ..pane.clone()
        });
        projected.agents.push(ClientShellAgent {
            pane_id,
            workspace_id,
            tab_id,
            name: Some(format!("agent {index}")),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: index as u64,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: index == 1,
        });
    }
    projected
}

fn search_state(count: usize) -> ClientShellState {
    let mut config = Config::default();
    config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(search_snapshot(count)));
    state.set_pane_surface(surface());
    state.compose(120, 50).unwrap();
    state
}

fn preview(state: &mut ClientShellState, bytes: &[u8]) {
    let result = state.handle_input_bytes(bytes);
    assert!(result.requests.is_empty(), "{bytes:?}");
    assert!(result.actions.is_empty(), "{bytes:?}");
}

fn search(state: &mut ClientShellState, section: u8, query: &str) {
    preview(state, b"\x02");
    preview(state, &[section]);
    preview(state, b"/");
    preview(state, query.as_bytes());
}

fn matches(state: &mut ClientShellState) -> bool {
    state
        .sidebar_search
        .refresh(&state.endpoints, &state.config)
}

fn visible_spaces(state: &ClientShellState) -> Vec<&str> {
    state
        .hits
        .workspaces
        .iter()
        .map(|hit| hit.workspace_id.as_str())
        .collect()
}

fn visible_agents(state: &ClientShellState) -> Vec<&str> {
    state
        .hits
        .agents
        .iter()
        .map(|(_, id)| id.as_str())
        .chain(
            state
                .hits
                .endpoint_agents
                .iter()
                .map(|(_, _, id)| id.as_str()),
        )
        .collect()
}

#[test]
fn sidebar_search_filters_spaces_and_ctrl_n_p_wrap_without_focusing() {
    let mut state = search_state(4);
    search(&mut state, b'w', "project");
    assert!(state.overlay.is_none());
    assert!(state.sidebar_search_focused());
    preview(&mut state, b"\x10");
    assert_eq!(
        state.navigate_workspace_id.as_ref().unwrap().workspace_id,
        "ws_4"
    );
    preview(&mut state, b"\x0e");
    assert_eq!(
        state.navigate_workspace_id.as_ref().unwrap().workspace_id,
        "ws_1"
    );
    preview(&mut state, b"\x1b[B");
    assert_eq!(
        state.navigate_workspace_id.as_ref().unwrap().workspace_id,
        "ws_2"
    );
    preview(&mut state, b"\x1b[A");
    assert_eq!(
        state.navigate_workspace_id.as_ref().unwrap().workspace_id,
        "ws_1"
    );
    preview(&mut state, b" 3");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_3"]);
    assert_eq!(
        visible_agents(&state),
        ["pane_1", "pane_2", "pane_3", "pane_4"]
    );
    assert_eq!(
        state
            .snapshot
            .as_ref()
            .unwrap()
            .focused_workspace_id
            .as_deref(),
        Some("ws_1")
    );
    let result = state.handle_input_bytes(b"\r");
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.sidebar_search.spaces.is_none());
    assert!(
        matches!(&result.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, crate::api::schema::Method::WorkspaceFocus(target) if target.workspace_id == "ws_3"))
    );
}

#[test]
fn sidebar_search_agents_keep_sidebar_order_and_ctrl_n_p_select_filtered_rows() {
    let mut state = search_state(4);
    search(&mut state, b'a', "agent");
    preview(&mut state, b"\x10");
    assert_eq!(state.navigate_agent.as_ref().unwrap().pane_id, "pane_4");
    preview(&mut state, b"\x0e");
    assert_eq!(state.navigate_agent.as_ref().unwrap().pane_id, "pane_1");
    preview(&mut state, b"\x0e");
    assert_eq!(state.navigate_agent.as_ref().unwrap().pane_id, "pane_2");
    preview(&mut state, b" 3");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_agents(&state), ["pane_3"]);
    assert_eq!(visible_spaces(&state), ["ws_1", "ws_2", "ws_3", "ws_4"]);
    let result = state.handle_input_bytes(b"\r");
    assert!(
        matches!(&result.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_3"))
    );
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.sidebar_search.agents.is_none());
}

#[test]
fn sidebar_search_esc_restores_list_then_leaves_navigation() {
    for section in [b'w', b'a'] {
        let mut state = search_state(3);
        search(&mut state, section, "zzzzz");
        let frame = state.compose(120, 50).unwrap();
        assert!(frame.cursor.is_none());
        assert!(if section == b'w' {
            visible_spaces(&state).is_empty()
        } else {
            visible_agents(&state).is_empty()
        });
        let buffer = frame.to_ratatui_buffer().unwrap();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains(if section == b'w' {
            "no matching spaces"
        } else {
            "no matching agents"
        }));
        preview(&mut state, b"\r");
        assert_eq!(state.mode, ClientShellMode::Navigate);
        preview(&mut state, b"\x1b");
        assert_eq!(state.mode, ClientShellMode::Navigate);
        state.compose(120, 50).unwrap();
        assert_eq!(visible_spaces(&state).len(), 3);
        assert_eq!(visible_agents(&state).len(), 3);
        preview(&mut state, b"\x1b");
        assert_eq!(state.mode, ClientShellMode::Terminal);
    }
}

#[test]
fn sidebar_search_paste_editing_and_section_queries_are_independent() {
    let mut state = search_state(3);
    search(&mut state, b'w', "");
    assert!(state.modal_paste_target_active());
    let result = state.handle_raw_events(vec![RawInputEvent::Paste("project\n3".into())]);
    assert!(result.requests.is_empty() && result.actions.is_empty());
    assert_eq!(
        state.sidebar_search.spaces.as_ref().unwrap().query.as_str(),
        "project 3"
    );
    preview(&mut state, b"\t");
    preview(&mut state, b"/");
    preview(&mut state, b"agent 2");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_3"]);
    assert_eq!(visible_agents(&state), ["pane_2"]);
    preview(&mut state, b"\x1b[Z");
    assert_eq!(state.navigate_section, SidebarNavSection::Spaces);
    preview(&mut state, b"\x15");
    preview(&mut state, b"project 1");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_1"]);
    assert_eq!(visible_agents(&state), ["pane_2"]);
    preview(&mut state, b"\x1b");
    preview(&mut state, b"\x1b");
    assert!(state.sidebar_search.spaces.is_none() && state.sidebar_search.agents.is_none());
}

fn composed_text(state: &mut ClientShellState) -> String {
    let frame = state.compose(120, 50).unwrap();
    frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn sidebar_search_mouse_controls_clear_and_open_result() {
    for (section, key) in [
        (SidebarNavSection::Spaces, b'w'),
        (SidebarNavSection::Agents, b'a'),
    ] {
        let mut state = search_state(3);
        search(&mut state, key, "3");
        let text = composed_text(&mut state);
        assert!(!text.contains("[/]"));
        assert!(text.contains(" / 3"), "{text}");
        let click = |state: &mut ClientShellState, rect: Rect| {
            state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::empty(),
            })])
        };
        let clear = if section == SidebarNavSection::Spaces {
            state.hits.spaces_search_clear
        } else {
            state.hits.agents_search_clear
        };
        assert!(!clear.is_empty());
        let outcome = click(&mut state, clear);
        assert!(outcome.actions.is_empty());
        state.compose(120, 50).unwrap();
        assert!(state.sidebar_search.section(section).is_none());
        assert_eq!(visible_spaces(&state).len(), 3);
        assert_eq!(visible_agents(&state).len(), 3);
        preview(&mut state, b"/");
        preview(&mut state, b"3");
        state.compose(120, 50).unwrap();
        if section == SidebarNavSection::Agents {
            let rect = state.hits.agents[0].0;
            let outcome = click(&mut state, rect);
            assert!(!outcome.actions.is_empty());
            assert_eq!(state.mode, ClientShellMode::Terminal);
            assert!(state.sidebar_search.agents.is_none());
        }
    }
}

#[test]
fn sidebar_search_scroll_reveals_selection_and_handles_live_removal() {
    let mut state = search_state(22);
    for section in [b'w', b'a'] {
        search(&mut state, section, "2");
        for _ in 0..4 {
            preview(&mut state, b"\x0e");
        }
        state.compose(100, 24).unwrap();
        if section == b'w' {
            let selected = &state.navigate_workspace_id.as_ref().unwrap().workspace_id;
            assert!(visible_spaces(&state).contains(&selected.as_str()));
        } else {
            let selected = &state.navigate_agent.as_ref().unwrap().pane_id;
            assert!(visible_agents(&state).contains(&selected.as_str()));
        }
        preview(&mut state, b"\x1b");
        preview(&mut state, b"\x1b");
    }
    search(&mut state, b'a', "agent 22");
    let mut updated = search_snapshot(21);
    updated.revision = 2;
    state.set_snapshot(Box::new(updated));
    state.refresh_sidebar_search();
    assert!(state.navigate_agent.is_none());
    preview(&mut state, b"\r");
    assert_eq!(state.mode, ClientShellMode::Navigate);
}

fn hidden_metadata_state() -> ClientShellState {
    let mut state = search_state(3);
    let mut projected = search_snapshot(3);
    projected.workspaces[0].label = "alpha".into();
    projected.workspaces[0].new_workspace_cwd = "/home/me/dotfiles".into();
    projected.workspaces[0].branch = Some("main".into());
    projected.agents[0].terminal_title = Some("dotfiles".into());
    projected.agents[0].terminal_title_stripped = Some("dotfiles".into());
    state.set_snapshot(Box::new(projected));
    state.compose(120, 50).unwrap();
    state
}

fn matching_spaces(query: &str) -> Vec<String> {
    let mut state = hidden_metadata_state();
    search(&mut state, b'w', query);
    state.compose(120, 50).unwrap();
    visible_spaces(&state)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn matching_agents(query: &str) -> Vec<String> {
    let mut state = hidden_metadata_state();
    search(&mut state, b'a', query);
    state.compose(120, 50).unwrap();
    visible_agents(&state)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn sidebar_search_matches_only_text_the_rows_draw() {
    assert!(matching_spaces("dotfiles").is_empty());
    assert_eq!(matching_spaces("alpha"), ["ws_1"]);
    assert!(matching_agents("dotfiles").is_empty());
    assert_eq!(matching_agents("agent 1"), ["pane_1"]);
}

#[test]
fn sidebar_search_rescoring_follows_the_configured_rows() {
    let mut state = search_state(3);
    search(&mut state, b'a', "local");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_agents(&state).len(), 3);
    let config: Config =
        toml::from_str("[ui.sidebar.agents]\nrows = [[\"agent\"], [\"workspace\"]]").unwrap();
    state.config = ClientShellConfig::from_config(&config);
    state.compose(120, 50).unwrap();
    assert!(visible_agents(&state).is_empty());
}

#[test]
fn sidebar_search_agent_view_is_intersected_not_replaced() {
    let mut state = search_state(3);
    let mut projected = search_snapshot(3);
    projected.agent_view_label = Some("selected agents".into());
    projected.agent_order = vec!["pane_2".into()];
    state.set_snapshot(Box::new(projected));
    search(&mut state, b'a', "agent");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_agents(&state), ["pane_2"]);
    preview(&mut state, b" 3");
    state.compose(120, 50).unwrap();
    assert!(visible_agents(&state).is_empty());
    assert!(state.navigate_agent.is_none());
}

#[test]
fn sidebar_search_cross_machine_ids_and_offline_targets_stay_safe() {
    use crate::client::endpoint::{ProfileId, SavedSshEndpoint};
    let mut state = search_state(3);
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let endpoint = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint, ClientEndpointStatus::Online);
    let mut projected = search_snapshot(3);
    projected.boot_id = "remote-boot".into();
    projected.workspaces[1].label = "build project 2".into();
    state.set_endpoint_snapshot(&endpoint, Box::new(projected));
    search(&mut state, b'a', "build agent 2");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_agents(&state), ["pane_2"]);
    assert_eq!(state.hits.endpoint_agents[0].1, endpoint);
    assert_eq!(state.navigate_agent.as_ref().unwrap().endpoint_id, endpoint);
    let result = state.handle_input_bytes(b"\r");
    assert!(
        matches!(&result.actions[..], [ClientShellAction::ActivateEndpoint { endpoint_id, target: Some(ClientEndpointFocusTarget::Pane(pane_id)) }]
        if endpoint_id == &endpoint && pane_id == "pane_2")
    );
    search(&mut state, b'w', "build");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_2"]);
    assert_eq!(state.hits.workspaces[0].endpoint_id, endpoint);
    state.set_endpoint_status(&endpoint, ClientEndpointStatus::Reconnecting);
    state.refresh_sidebar_search();
    assert!(state.navigate_workspace_id.is_none());
    preview(&mut state, b"\r");
    assert_eq!(state.mode, ClientShellMode::Navigate);
}

#[test]
fn sidebar_search_reveals_collapsed_worktrees_without_changing_collapse_state() {
    let mut state = search_state(3);
    let mut projected = search_snapshot(3);
    for (index, ws) in projected.workspaces.iter_mut().enumerate().take(2) {
        ws.worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: index == 1,
        });
    }
    state.set_snapshot(Box::new(projected));
    state.collapsed_groups.insert("repo".into());
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_1", "ws_3"]);
    // A linked worktree row draws its branch, not the workspace label.
    search(&mut state, b'w', "task-2");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_2"]);
    assert_eq!(
        state.navigate_workspace_id.as_ref().unwrap().workspace_id,
        "ws_2"
    );
    preview(&mut state, b"\x1b");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_1", "ws_3"]);
    assert!(state.collapsed_groups.contains("repo"));
}

#[test]
fn sidebar_search_resize_to_mobile_does_not_keep_a_hidden_filter() {
    let mut state = search_state(3);
    search(&mut state, b'w', "project 2");
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state), ["ws_2"]);
    state.compose(40, 24).unwrap();
    assert!(state.mobile_layout_active());
    assert!(state.sidebar_search.spaces.is_none());
    state.compose(120, 50).unwrap();
    assert_eq!(visible_spaces(&state).len(), 3);
}

#[test]
fn sidebar_search_reuses_matches_until_snapshot_metadata_changes() {
    let mut state = search_state(3);
    search(&mut state, b'a', "special");
    assert!(!matches(&mut state));
    let mut updated = search_snapshot(3);
    updated.revision += 1;
    updated.agents[1].name = Some("special".into());
    state.set_snapshot(Box::new(updated));
    state.refresh_sidebar_search();
    assert_eq!(state.navigate_agent.as_ref().unwrap().pane_id, "pane_2");
    assert!(!matches(&mut state));
    assert!(state
        .sidebar_search
        .agents
        .as_ref()
        .unwrap()
        .includes(&ClientEndpointId::Local, "pane_2"));
}

#[test]
#[ignore = "manual sidebar search composition scaling profile"]
fn sidebar_search_render_scale_profile() {
    for count in [1, 15, 52] {
        for section in [b'w', b'a'] {
            for query in [
                None,
                Some(if section == b'w' { "project" } else { "agent" }),
            ] {
                let mut state = search_state(count);
                if let Some(query) = query {
                    search(&mut state, section, query);
                }
                for _ in 0..20 {
                    std::hint::black_box(state.compose(106, 30).unwrap());
                }
                let start = std::time::Instant::now();
                for _ in 0..1000 {
                    std::hint::black_box(state.compose(106, 30).unwrap());
                }
                eprintln!(
                    "sidebar search: {count} panes, section={}, query={query:?}, {:.1} us/frame",
                    section as char,
                    start.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
    }
}

#[test]
fn sidebar_search_typed_shortcuts_are_text_and_shifted_slash_opens_search() {
    let mut state = search_state(3);
    preview(&mut state, b"\x02w");
    let key = crate::input::TerminalKey::new(KeyCode::Char('7'), KeyModifiers::SHIFT)
        .with_shifted_codepoint('/' as u32);
    let result = state.handle_raw_events(vec![RawInputEvent::Key(key)]);
    assert!(result.actions.is_empty());
    assert!(state.sidebar_search_focused());
    preview(&mut state, b"jkl123/");
    assert_eq!(
        state.sidebar_search.spaces.as_ref().unwrap().query.as_str(),
        "jkl123/"
    );
    assert_eq!(
        state
            .snapshot
            .as_ref()
            .unwrap()
            .focused_workspace_id
            .as_deref(),
        Some("ws_1")
    );
}
