use super::*;

fn two_agent_snapshot() -> ClientShellSnapshot {
    let mut projected = snapshot();
    let mut second_pane = projected.panes[0].clone();
    second_pane.pane_id = "pane_2".into();
    second_pane.focused = false;
    projected.panes.push(second_pane);
    projected.agents = vec![
        ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("pi one".into()),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: crate::api::schema::AgentStatus::Working,
            state_change_seq: 10,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
        },
        ClientShellAgent {
            pane_id: "pane_2".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("pi two".into()),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: crate::api::schema::AgentStatus::Idle,
            state_change_seq: 20,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
        },
    ];
    projected
}

fn agent_nav_state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_agent_snapshot()));
    state.set_pane_surface(surface());
    state
}

fn preview_key(state: &mut ClientShellState, bytes: &[u8]) {
    let outcome = state.handle_input_bytes(bytes);
    assert!(outcome.actions.is_empty(), "{bytes:?}");
    assert!(outcome.requests.is_empty(), "{bytes:?}");
    assert!(outcome.repaint, "{bytes:?}");
}

fn enter_navigation(state: &mut ClientShellState) {
    preview_key(state, &[0x02]);
    preview_key(state, b"w");
    assert_eq!(state.mode, ClientShellMode::Navigate);
}

fn previewed_pane(state: &ClientShellState) -> Option<String> {
    state.navigate_agent.clone().map(|target| target.pane_id)
}

fn enter_agents_navigation(state: &mut ClientShellState) {
    preview_key(state, &[0x02]);
    preview_key(state, b"a");
    assert_eq!(state.mode, ClientShellMode::Navigate);
    assert_eq!(state.navigate_section, SidebarNavSection::Agents);
}

#[test]
fn prefix_a_enters_navigation_with_agents_section_active() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_navigation(&mut state);
    assert_eq!(previewed_pane(&state).as_deref(), Some("pane_1"));
    preview_key(&mut state, b"\x1b[B");
    assert_eq!(previewed_pane(&state).as_deref(), Some("pane_2"));
    assert_eq!(state.mode, ClientShellMode::Navigate);
}

#[test]
fn prefix_w_still_enters_navigation_with_spaces_section_active() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_navigation(&mut state);
    assert_eq!(state.navigate_section, SidebarNavSection::Spaces);
}

fn enter_agents_section(state: &mut ClientShellState) {
    enter_navigation(state);
    assert_eq!(state.navigate_section, SidebarNavSection::Spaces);
    assert_eq!(previewed_pane(state).as_deref(), Some("pane_1"));
    preview_key(state, b"\t");
    assert_eq!(state.mode, ClientShellMode::Navigate);
    assert_eq!(state.navigate_section, SidebarNavSection::Agents);
}

#[test]
fn tab_switches_section_and_down_moves_agent_preview_without_focusing() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_section(&mut state);
    preview_key(&mut state, b"\x1b[B");
    assert_eq!(previewed_pane(&state).as_deref(), Some("pane_2"));
    assert_eq!(state.mode, ClientShellMode::Navigate);
    assert_eq!(
        state
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone())
            .as_deref(),
        Some("pane_1")
    );
}

#[test]
fn backtab_returns_to_spaces_section() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_section(&mut state);
    preview_key(&mut state, b"\x1b[Z");
    assert_eq!(state.navigate_section, SidebarNavSection::Spaces);
    assert_eq!(state.mode, ClientShellMode::Navigate);
}

#[test]
fn enter_focuses_previewed_agent_and_exits_navigation() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_section(&mut state);
    preview_key(&mut state, b"\x1b[B");
    let enter = state.handle_input_bytes(b"\r");
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.navigate_agent.is_none());
    assert!(state.navigate_workspace_id.is_none());
    assert_eq!(state.navigate_section, SidebarNavSection::Spaces);
    let [ClientShellAction::Endpoint { request, .. }] = &enter.actions[..] else {
        panic!("enter should focus the previewed agent");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
    ));
}

#[test]
fn esc_clears_agent_preview_without_focusing() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_section(&mut state);
    preview_key(&mut state, b"\x1b[B");
    let esc = state.handle_input_bytes(b"\x1b");
    assert!(esc.actions.is_empty() && esc.requests.is_empty());
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.navigate_agent.is_none());
    assert_eq!(state.navigate_section, SidebarNavSection::Spaces);
    assert_eq!(
        state
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone())
            .as_deref(),
        Some("pane_1")
    );
}

#[test]
fn digit_in_agents_section_focuses_indexed_agent() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_section(&mut state);
    let two = state.handle_input_bytes(b"2");
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.navigate_agent.is_none());
    let [ClientShellAction::Endpoint { request, .. }] = &two.actions[..] else {
        panic!("digit should focus the indexed agent");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
    ));
}

#[test]
fn agent_preview_wraps_and_empty_list_stays_safe() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_section(&mut state);
    preview_key(&mut state, b"\x1b[A");
    assert_eq!(previewed_pane(&state).as_deref(), Some("pane_2"));

    let mut empty = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    empty.set_snapshot(Box::new(snapshot()));
    empty.set_pane_surface(surface());
    empty.compose(100, 28).unwrap();
    enter_navigation(&mut empty);
    preview_key(&mut empty, b"\t");
    assert!(empty.navigate_agent.is_none());
    preview_key(&mut empty, b"\x1b[B");
    assert!(empty.navigate_agent.is_none());
    let enter = empty.handle_input_bytes(b"\r");
    assert!(enter.actions.is_empty() && enter.requests.is_empty());
    assert_eq!(empty.mode, ClientShellMode::Terminal);
}

#[test]
fn agent_preview_highlights_selected_row_not_focused_row() {
    let mut state = agent_nav_state();
    state.compose(100, 28).unwrap();
    enter_agents_section(&mut state);
    preview_key(&mut state, b"\x1b[B");
    let buffer = state.compose(100, 28).unwrap().to_ratatui_buffer().unwrap();
    let palette = &state.config.palette;
    let mut selection = palette.selection_bg;
    if selection == ratatui::style::Color::Reset {
        selection = palette.active_row_bg;
    }
    let selected = state
        .hits
        .agents
        .iter()
        .find(|(_, pane_id)| pane_id == "pane_2")
        .map(|(rect, _)| *rect)
        .expect("previewed agent hit");
    let focused = state
        .hits
        .agents
        .iter()
        .find(|(_, pane_id)| pane_id == "pane_1")
        .map(|(rect, _)| *rect)
        .expect("focused agent hit");
    assert_eq!(buffer[(selected.x, selected.y)].bg, selection);
    assert_eq!(buffer[(focused.x, focused.y)].bg, palette.active_row_bg);
}
