use super::*;

#[path = "workspace_navigation.rs"]
mod workspace_navigation;
use crate::client::endpoint::{
    ClientEndpointId, ClientEndpointStatus, ProfileId, SavedSshEndpoint,
};
use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

fn remote_profile() -> SavedSshEndpoint {
    SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    }
}

fn agent(
    name: &str,
    status: crate::api::schema::AgentStatus,
    state_change_seq: u64,
) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some(name.into()),
        display_agent: None,
        agent: Some("pi".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: true,
    }
}

fn current_workspace_view() -> crate::api::schema::AgentViewSetParams {
    use crate::api::schema::{
        AgentViewBuiltinField, AgentViewContext, AgentViewField, AgentViewFilter, AgentViewValue,
    };

    crate::api::schema::AgentViewSetParams {
        source: "example.views".into(),
        label: Some("current space".into()),
        filter: Some(AgentViewFilter::Eq {
            field: AgentViewField::Builtin(AgentViewBuiltinField::WorkspaceId),
            value: AgentViewValue::Context {
                context: AgentViewContext::CurrentWorkspaceId,
            },
        }),
        sort: Vec::new(),
    }
}

fn state_with_remote() -> (ClientShellState, ClientEndpointId) {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.workspaces[0].label = "remote-workspace".into();
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));
    (state, endpoint_id)
}

#[cfg(windows)]
#[test]
fn system_notification_clicks_keep_endpoint_and_boot_identity() {
    let (mut state, remote) = state_with_remote();
    state.config.toast_delivery = crate::config::ToastDelivery::System;
    state.config.toast_delay_seconds = 0;
    state.outer_focused = Some(false);
    for endpoint_id in [ClientEndpointId::Local, remote.clone()] {
        let (effects, _) = state.receive_notification(
            &endpoint_id,
            SemanticNotification {
                kind: SemanticNotificationKind::Custom,
                title: "test".into(),
                body: None,
                sound: None,
                agent: None,
                workspace_id: Some("ws_1".into()),
                tab_id: Some("tab_1".into()),
                pane_id: Some("pane_1".into()),
                position: None,
            },
            std::time::Instant::now(),
        );
        let [ClientShellNotificationEffect::System {
            target: Some(target),
            ..
        }] = effects.as_slice()
        else {
            panic!("system effect must retain notification target");
        };
        let target = target.clone();
        assert_eq!(target.endpoint_id, endpoint_id);
        let outcome = state.activate_system_notification(target.clone());
        assert!(
            !outcome.actions.is_empty(),
            "a current target must navigate"
        );
        if endpoint_id == remote {
            assert!(
                matches!(&outcome.actions[..], [ClientShellAction::ActivateEndpoint {
                endpoint_id: id, target: Some(ClientEndpointFocusTarget::Notification { pane_id, boot_id }),
            }] if id == &remote && pane_id == "pane_1" && boot_id == "remote-boot")
            );
        }
        state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Reconnecting);
        assert!(state
            .activate_system_notification(target.clone())
            .actions
            .is_empty());
        state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
        assert!(
            !state
                .activate_system_notification(target.clone())
                .actions
                .is_empty(),
            "same-boot reconnect remains valid"
        );
        let endpoint = state
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.endpoint_id == endpoint_id)
            .unwrap();
        let snapshot = endpoint.snapshot.as_mut().unwrap();
        snapshot.boot_id = "replacement-boot".into();
        assert!(
            state
                .activate_system_notification(target.clone())
                .actions
                .is_empty(),
            "same pane ID from another boot must not navigate"
        );
        let endpoint = state
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.endpoint_id == endpoint_id)
            .unwrap();
        let snapshot = endpoint.snapshot.as_mut().unwrap();
        snapshot.boot_id = target.boot_id.clone();
        snapshot.panes.clear();
        assert!(
            state
                .activate_system_notification(target.clone())
                .actions
                .is_empty(),
            "closed pane must not navigate"
        );
        if endpoint_id == remote {
            state.set_endpoint_catalog(&[]);
            assert!(
                state
                    .activate_system_notification(target)
                    .actions
                    .is_empty(),
                "removed profile must not navigate"
            );
        }
    }
}

#[test]
fn machine_diagnostic_badge_reopens_notice_without_collapsing_machine() {
    let (mut state, id) = state_with_remote();
    state.set_endpoint_status(&id, ClientEndpointStatus::Attention);
    state.set_machine_diagnostic(&id, "Permission denied (keyboard-interactive)".into());
    for _ in 0..2 {
        state.compose(120, 40).unwrap();
        let hit = state
            .hits
            .machines
            .iter()
            .find(|hit| hit.endpoint_id == id)
            .unwrap();
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.status_badge.x,
            row: hit.status_badge.y,
            modifiers: KeyModifiers::NONE,
        };
        let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(mouse)]);
        assert!(outcome.repaint);
        assert!(!state.collapsed_endpoints.contains(&id));
        let notice = state.visible_endpoint_notice.take().unwrap();
        assert!(notice.body.contains("Permission denied"));
        assert!(notice
            .title
            .contains("herdr machine reconnect 0123456789abcdef0123456789abcdef"));
    }
    state.set_endpoint_status(&id, ClientEndpointStatus::Online);
    state.compose(120, 40).unwrap();
    assert!(!state.machine_diagnostics.required_for(
        state
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == id)
            .unwrap()
    ));
}

fn state_with_scrollable_agents() -> (ClientShellState, ClientEndpointId) {
    let (mut state, remote) = state_with_remote();
    for endpoint_id in [ClientEndpointId::Local, remote.clone()] {
        let mut projection = state
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == endpoint_id)
            .unwrap()
            .snapshot
            .clone()
            .unwrap();
        projection.agents = (0..8)
            .map(|index| ClientShellAgent {
                pane_id: format!("pane_{}", index + 1),
                focused: index == 0,
                ..agent(&format!("agent {index}"), AgentStatus::Idle, 1)
            })
            .collect();
        projection.panes = projection
            .agents
            .iter()
            .map(|agent| ClientShellPane {
                pane_id: agent.pane_id.clone(),
                focused: agent.focused,
                ..projection.panes[0].clone()
            })
            .collect();
        state.set_endpoint_snapshot(&endpoint_id, projection);
    }
    state.compose(100, 28).unwrap();
    state.agent_scroll = 6;
    state.compose(100, 28).unwrap();
    assert_eq!(state.agent_scroll, 6);
    (state, remote)
}

#[test]
fn agent_navigation_reveals_offscreen_targets() {
    use crate::input::KeybindAction;

    for action in [
        KeybindAction::NextAgent,
        KeybindAction::PreviousAgent,
        KeybindAction::FocusAgent(0),
    ] {
        let (mut state, remote) = state_with_scrollable_agents();
        let (endpoint_id, pane_id) = match action {
            KeybindAction::NextAgent => (ClientEndpointId::Local, "pane_2"),
            KeybindAction::PreviousAgent => (remote, "pane_8"),
            _ => (ClientEndpointId::Local, "pane_1"),
        };
        state.agent_scroll = if action == KeybindAction::PreviousAgent {
            0
        } else {
            state.hits.agent_max_scroll
        };
        state.compose(100, 28).unwrap();
        assert!(!state
            .hits
            .endpoint_agents
            .iter()
            .any(|(_, endpoint, pane)| { endpoint == &endpoint_id && pane == pane_id }));

        let mut outcome = ClientShellInput::default();
        assert!(state.handle_endpoint_navigation(action, &mut outcome));
        assert!(outcome.repaint, "agent navigation must request a frame");
        if endpoint_id != state.active_endpoint_id {
            assert!(state.activate_endpoint_projection(&endpoint_id));
        }
        state.compose(100, 28).unwrap();
        assert!(
            state
                .hits
                .endpoint_agents
                .iter()
                .any(|(_, endpoint, pane)| { endpoint == &endpoint_id && pane == pane_id }),
            "{action:?} must reveal the selected agent"
        );
    }
}

#[test]
fn agent_navigation_reveals_target_using_destination_sort() {
    use crate::api::schema::{
        AgentViewBuiltinSortField, AgentViewSort, AgentViewSortField, AgentViewSortOrder,
    };

    let (mut state, remote) = state_with_scrollable_agents();
    for (endpoint_id, base) in [(ClientEndpointId::Local, 0), (remote.clone(), 8)] {
        let mut projection = state
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == endpoint_id)
            .unwrap()
            .snapshot
            .clone()
            .unwrap();
        for (index, agent) in projection.agents.iter_mut().enumerate() {
            agent.state_change_seq = base + index as u64;
        }
        if endpoint_id == remote {
            projection.agent_view_label = Some("recent".into());
        }
        state.set_endpoint_snapshot(&endpoint_id, projection);
    }
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, None);
    let mut view = current_workspace_view();
    view.label = Some("recent".into());
    view.filter = None;
    view.sort = vec![AgentViewSort {
        field: AgentViewSortField::Builtin(AgentViewBuiltinSortField::StateChangeSeq),
        order: AgentViewSortOrder::Desc,
    }];
    state.set_test_endpoint_agent_view(&remote, Some(view));
    state.compose(100, 28).unwrap();

    let mut outcome = ClientShellInput::default();
    assert!(state
        .handle_endpoint_navigation(crate::input::KeybindAction::FocusAgent(15), &mut outcome,));
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id,
            target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
        }] if endpoint_id == &remote && pane_id == "pane_8"
    ));
    // A superseded handoff restores its source before activating the new target.
    assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
    state.compose(100, 28).unwrap();
    assert!(state.activate_endpoint_projection(&remote));
    state.compose(100, 28).unwrap();
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .any(|(_, endpoint, pane)| { endpoint == &remote && pane == "pane_8" }));
}

#[test]
fn agent_navigation_reveal_is_cancelled_by_another_selection() {
    for select_pane in [false, true] {
        let (mut state, remote) = state_with_scrollable_agents();
        let scroll = state.agent_scroll;
        let mut outcome = ClientShellInput::default();
        assert!(state
            .handle_endpoint_navigation(crate::input::KeybindAction::PreviousAgent, &mut outcome,));
        assert_eq!(state.agent_scroll, scroll);
        if select_pane {
            assert!(state.focus_or_activate(
                remote.clone(),
                ClientEndpointFocusTarget::Pane("pane_1".into()),
                &mut outcome,
            ));
        } else {
            assert!(state.activate_endpoint(remote.clone(), &mut outcome));
        }
        assert!(state.activate_endpoint_projection(&remote));
        state.compose(100, 28).unwrap();
        assert_eq!(state.agent_scroll, scroll);
    }
}

#[test]
fn agent_navigation_keeps_scroll_when_target_is_visible() {
    let (mut state, _) = state_with_scrollable_agents();
    let (_, endpoint_id, pane_id) = state.hits.endpoint_agents[1].clone();
    let targets = super::super::aggregate_navigation::online_agent_targets(
        &state.endpoints,
        &state.active_endpoint_id,
        state.config.agent_panel_sort,
    );
    let index = targets
        .iter()
        .position(|target| target.endpoint_id == endpoint_id && target.pane_id == pane_id)
        .unwrap();
    let scroll = state.agent_scroll;
    assert!(state.handle_endpoint_navigation(
        crate::input::KeybindAction::FocusAgent(index),
        &mut ClientShellInput::default(),
    ));
    state.compose(100, 28).unwrap();
    assert_eq!(state.agent_scroll, scroll);
}

#[test]
fn switching_machines_preserves_aggregate_agent_scroll_and_visible_rows() {
    let (mut state, remote) = state_with_scrollable_agents();
    for endpoint_id in [remote.clone(), ClientEndpointId::Local, remote] {
        let visible = state.hits.endpoint_agents.clone();
        let (rect, _, pane_id) = visible
            .iter()
            .find(|(_, endpoint, _)| endpoint == &endpoint_id)
            .expect("destination agent remains visible");
        let click = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 2,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })]);
        assert!(matches!(
            click.actions.as_slice(),
            [ClientShellAction::ActivateEndpoint {
                endpoint_id: target,
                target: Some(ClientEndpointFocusTarget::Pane(target_pane)),
            }] if target == &endpoint_id && target_pane == pane_id
        ));

        state.workspace_scroll = 3;
        state.tab_scroll = 2;
        assert!(state.activate_endpoint_projection(&endpoint_id));
        assert_eq!(state.agent_scroll, 6);
        assert_eq!(state.workspace_scroll, 0);
        assert_eq!(state.tab_scroll, 0);
        assert!(state.pane_surface.is_none());

        let mut next_surface = surface();
        next_surface.boot_id = state.endpoint_boot_id(&endpoint_id).unwrap().into();
        state.set_pane_surface(next_surface);
        state.compose(100, 28).unwrap();
        assert_eq!(state.agent_scroll, 6);
        assert_eq!(state.hits.endpoint_agents, visible);
    }
}

#[test]
fn local_agent_click_can_cancel_a_pending_remote_switch() {
    for reconnecting in [false, true] {
        let (mut state, remote) = state_with_scrollable_agents();
        assert!(state.activate_endpoint(remote, &mut ClientShellInput::default()));
        if reconnecting {
            state.mark_endpoint_disconnected(&ClientEndpointId::Local);
        }
        state.compose(100, 28).unwrap();
        let (rect, _, pane_id) = state
            .hits
            .endpoint_agents
            .iter()
            .find(|(_, endpoint, _)| endpoint.is_local())
            .unwrap()
            .clone();
        let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 2,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })]);
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::ActivateEndpoint {
            endpoint_id: ClientEndpointId::Local,
            target: Some(ClientEndpointFocusTarget::Pane(target)),
        }] if target == &pane_id)
        );
    }
}

#[test]
fn aggregate_agent_scroll_still_clamps_when_rows_shrink_on_activation() {
    let (mut state, remote) = state_with_scrollable_agents();
    for endpoint_id in [ClientEndpointId::Local, remote.clone()] {
        let mut projection = state
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == endpoint_id)
            .unwrap()
            .snapshot
            .clone()
            .unwrap();
        projection.revision += 1;
        projection.agents.truncate(1);
        state.set_endpoint_snapshot(&endpoint_id, projection);
    }
    assert!(state.activate_endpoint_projection(&remote));
    state.compose(100, 28).unwrap();
    assert_eq!(state.agent_scroll, 0);
    assert_eq!(state.hits.agent_max_scroll, 0);
    assert_eq!(state.hits.endpoint_agents.len(), 2);
}

#[test]
fn same_machine_reboot_still_resets_agent_scroll() {
    let (mut state, _) = state_with_scrollable_agents();
    let mut projection = state.snapshot.clone().unwrap();
    projection.boot_id = "restarted-local".into();
    state.cache_endpoint_snapshot(&ClientEndpointId::Local, projection);
    assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
    assert_eq!(state.agent_scroll, 0);
}

#[test]
fn switching_machines_from_copy_mode_restores_terminal_input() {
    let (mut state, remote) = state_with_remote();
    let mut local_surface = surface();
    local_surface.panes[0].scroll = Some(crate::protocol::PaneSurfaceScrollMetrics {
        offset_from_bottom: 0,
        max_offset_from_bottom: 20,
        viewport_rows: 2,
    });
    state.set_pane_surface(local_surface);
    state.compose(100, 28).unwrap();
    assert!(state.enter_copy_mode(&mut ClientShellInput::default()));
    assert_eq!(state.mode, ClientShellMode::Copy);

    assert!(state.activate_endpoint_projection(&remote));
    let mut remote_surface = surface();
    remote_surface.boot_id = "remote-boot".into();
    state.set_pane_surface(remote_surface);
    state.compose(100, 28).unwrap();

    assert!(state.copy_mode.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
    let input = state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
    ))]);
    assert!(matches!(
        input.requests.as_slice(),
        [ClientMessage::ClientShellPaneInput { pane_id, events }]
            if pane_id == "pane_1" && events.len() == 1
    ));
}

#[test]
fn live_catalog_rename_preserves_snapshot_and_disable_reenable_clears_it() {
    let (mut state, remote) = state_with_remote();
    let mut profile = remote_profile();
    profile.label = "Renamed".into();
    state.set_endpoint_catalog(&[profile.clone()]);
    assert_eq!(state.endpoint_label(&remote), "Renamed");
    assert!(state.endpoint_is_online(&remote));
    assert_eq!(state.endpoint_boot_id(&remote), Some("remote-boot"));
    profile.enabled = false;
    state.set_endpoint_catalog(&[profile.clone()]);
    assert_eq!(
        state.endpoint_status(&remote),
        Some(ClientEndpointStatus::Disabled)
    );
    assert!(!state.endpoint_has_snapshot(&remote));
    profile.enabled = true;
    state.set_endpoint_catalog(&[profile]);
    assert_eq!(
        state.endpoint_status(&remote),
        Some(ClientEndpointStatus::Connecting)
    );
    assert!(!state.endpoint_has_snapshot(&remote));
}

#[test]
fn live_catalog_active_removal_does_not_retain_remote_projection_or_input() {
    let (mut state, remote) = state_with_remote();
    assert!(state.activate_endpoint_projection(&remote));
    state.set_pane_surface(surface());
    state.mode = ClientShellMode::Prefix;
    state.overlay = Some(ClientShellOverlay::Onboarding);
    state.select_unavailable_local();
    state.retire_endpoint(&remote);
    state.set_endpoint_catalog(&[]);
    assert!(state.endpoint_is_active(&ClientEndpointId::Local));
    assert!(state.snapshot.is_none());
    assert!(state.pane_surface.is_none());
    assert!(state.pending_pane_surface.is_none());
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.endpoint_has_snapshot(&ClientEndpointId::Local));
    let frame = state.compose(100, 30).unwrap();
    let buffer = frame.to_ratatui_buffer().unwrap();
    let text = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!text.contains("remote-workspace"));
}

#[test]
fn machine_navigation_does_not_require_a_local_snapshot_or_surface() {
    for (cols, rows) in [(100, 28), (36, 18)] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let profile = remote_profile();
        let remote = ClientEndpointId::Ssh(profile.id.clone());
        state.set_endpoint_catalog(&[profile]);
        state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Reconnecting);
        state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
        state.set_endpoint_snapshot(&remote, Box::new(snapshot()));
        assert!(state.snapshot.is_none());
        assert!(state.pane_surface.is_none());
        let frame = state
            .compose(cols, rows)
            .expect("connection chrome without Local");
        let local = state
            .hits
            .machines
            .iter()
            .find(|hit| hit.endpoint_id.is_local())
            .unwrap()
            .rect;
        let buffer = frame.to_ratatui_buffer().unwrap();
        let local_row = (local.x..local.right())
            .map(|x| buffer[(x, local.y)].symbol())
            .collect::<String>();
        assert!(!local_row.contains("reconnecting"));
        assert!(
            !local_row.contains('◐'),
            "Local never gets a connection badge"
        );
        let hit = state
            .hits
            .machines
            .iter()
            .find(|hit| hit.endpoint_id == remote)
            .unwrap()
            .rect;
        let mut outcome = ClientShellInput::default();
        state.handle_mouse(
            crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.x + 5,
                row: hit.y,
                modifiers: KeyModifiers::NONE,
            },
            &mut outcome,
        );
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::ActivateEndpoint { endpoint_id, .. }] if endpoint_id == &remote)
        );
        assert!(
            state.snapshot.is_none(),
            "selection is committed only by coherent activation"
        );
    }
}

#[test]
fn sidebar_renders_local_and_saved_ssh_endpoints_with_status() {
    let (mut state, _) = state_with_remote();
    let frame = state.compose(100, 28).expect("combined endpoint frame");
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
    assert!(text.contains("Local"));
    assert!(text.contains("Build"));
    assert!(text.contains("remote-workspace"));
    let local = state
        .hits
        .machines
        .iter()
        .find(|hit| hit.endpoint_id.is_local())
        .expect("local machine row")
        .rect;
    let remote = state
        .hits
        .machines
        .iter()
        .find(|hit| !hit.endpoint_id.is_local())
        .expect("remote machine row")
        .rect;
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    assert_ne!(buffer[(local.right() - 1, local.y)].symbol(), "●");
    assert_eq!(buffer[(remote.right() - 1, remote.y)].symbol(), "●");
    assert_eq!(
        buffer[(remote.right() - 1, remote.y)].fg,
        state.config.palette.green
    );

    state.sidebar_collapsed = true;
    let frame = state.compose(100, 28).expect("collapsed endpoint frame");
    let local = state
        .hits
        .machines
        .iter()
        .find(|hit| hit.endpoint_id.is_local())
        .expect("collapsed local machine row")
        .rect;
    let remote = state
        .hits
        .machines
        .iter()
        .find(|hit| !hit.endpoint_id.is_local())
        .expect("collapsed remote machine row")
        .rect;
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    assert_ne!(buffer[(local.right() - 1, local.y)].symbol(), "●");
    assert_eq!(buffer[(remote.right() - 1, remote.y)].symbol(), "●");
}

#[test]
fn saved_machine_preserves_endpoint_scoped_worktree_collapses() {
    fn add_worktree_group(snapshot: &mut ClientShellSnapshot, parent_id: &str, child_id: &str) {
        snapshot.workspaces[0].workspace_id = parent_id.into();
        snapshot.workspaces[0].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: false,
        });
        let mut child = snapshot.workspaces[0].clone();
        child.workspace_id = child_id.into();
        child.active_tab_id = format!("tab_{child_id}");
        child.number = 2;
        child.label = "feature".into();
        child.focused = false;
        child.agent_status = AgentStatus::Blocked;
        child.worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: true,
        });
        snapshot.workspaces.push(child);
    }

    let (mut state, remote_id) = state_with_remote();
    let mut local = snapshot();
    add_worktree_group(&mut local, "ws_1", "ws_2");
    state.set_snapshot(Box::new(local));
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.focused_workspace_id = Some("remote_ws_1".into());
    add_worktree_group(&mut remote, "remote_ws_1", "remote_ws_2");
    state.set_endpoint_snapshot(&remote_id, Box::new(remote));

    state.open_workspace_context_menu("ws_1".into(), 0, 0);
    let toggle_index = match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .position(|item| item.action == ClientContextMenuAction::ToggleGroup)
            .expect("collapse menu item"),
        _ => panic!("workspace context menu"),
    };
    state.activate_context_menu_item(toggle_index, &mut ClientShellInput::default());

    let frame = state
        .compose(100, 28)
        .expect("collapsed local worktree group");
    assert!(!state
        .hits
        .workspaces
        .iter()
        .any(|hit| { hit.endpoint_id == ClientEndpointId::Local && hit.workspace_id == "ws_2" }));
    assert!(state
        .hits
        .workspaces
        .iter()
        .any(|hit| hit.endpoint_id == remote_id && hit.workspace_id == "remote_ws_2"));
    let local_parent = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == ClientEndpointId::Local && hit.workspace_id == "ws_1")
        .expect("local parent workspace");
    let (local_toggle, key) = local_parent
        .group_toggle
        .as_ref()
        .expect("local worktree group marker");
    assert_eq!(key, "repo");
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    assert_eq!(buffer[(local_toggle.x, local_toggle.y)].symbol(), "▸");
    assert!((local_parent.rect.x..local_parent.rect.right())
        .any(|x| buffer[(x, local_parent.rect.y)].fg == state.config.palette.red));

    assert!(state.activate_endpoint_projection(&remote_id));
    let mut remote_surface = surface();
    remote_surface.boot_id = "remote-boot".into();
    state.set_pane_surface(remote_surface);
    state
        .compose(100, 28)
        .expect("active remote worktree group");
    let mut switch = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::SwitchWorkspace(1)),
        &mut switch,
    );
    assert!(matches!(
        &switch.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::WorkspaceFocus(target)
                    if target.workspace_id == "remote_ws_2"
            )
    ));
    let remote_parent = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == remote_id && hit.workspace_id == "remote_ws_1")
        .expect("visible remote worktree parent")
        .rect;
    let remote_child = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == remote_id && hit.workspace_id == "remote_ws_2")
        .expect("visible remote worktree child")
        .rect;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: remote_parent.x + 3,
        row: remote_parent.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: remote_child.x + 3,
        row: remote_child.bottom(),
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(matches!(
        state.chrome_drag,
        Some(ClientChromeDrag::Workspace {
            target: Some(_),
            ..
        })
    ));
    state.chrome_drag = None;
    state.workspace_press = None;

    let remote_toggle = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == remote_id && hit.workspace_id == "remote_ws_1")
        .and_then(|hit| hit.group_toggle.as_ref())
        .expect("remote worktree group marker")
        .0;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: remote_toggle.x,
        row: remote_toggle.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.compose(100, 28).expect("both groups collapsed");
    assert!(!state
        .hits
        .workspaces
        .iter()
        .any(|hit| { hit.endpoint_id == remote_id && hit.workspace_id == "remote_ws_2" }));

    let local_toggle = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == ClientEndpointId::Local && hit.workspace_id == "ws_1")
        .and_then(|hit| hit.group_toggle.as_ref())
        .expect("collapsed local group marker")
        .0;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: local_toggle.x,
        row: local_toggle.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.compose(100, 28).expect("only remote group collapsed");
    assert!(state
        .hits
        .workspaces
        .iter()
        .any(|hit| { hit.endpoint_id == ClientEndpointId::Local && hit.workspace_id == "ws_2" }));
    assert!(!state
        .hits
        .workspaces
        .iter()
        .any(|hit| { hit.endpoint_id == remote_id && hit.workspace_id == "remote_ws_2" }));
}

#[test]
fn expanded_machine_sidebar_reveals_newly_focused_workspace() {
    let (mut state, remote_id) = state_with_remote();
    let mut initial = snapshot();
    let template = initial.workspaces[0].clone();
    initial.workspaces = (1..=12)
        .map(|number| ClientShellWorkspace {
            workspace_id: format!("ws_{number}"),
            number,
            label: format!("space-{number}"),
            focused: number == 1,
            ..template.clone()
        })
        .collect();
    // Reuse workspace IDs across machines so revealing must be endpoint-scoped.
    let mut remote = initial.clone();
    remote.boot_id = "remote-boot".into();
    remote.workspaces.push(ClientShellWorkspace {
        workspace_id: "ws_13".into(),
        number: 13,
        focused: false,
        ..template.clone()
    });
    state.set_endpoint_snapshot(&remote_id, Box::new(remote));
    state.set_snapshot(Box::new(initial));
    state.compose(106, 20).expect("full machines sidebar");
    assert!(state.hits.workspace_max_scroll > 0);

    let mut update = state.snapshot.as_deref().expect("snapshot").clone();
    update.revision = 2;
    update.workspaces.push(ClientShellWorkspace {
        workspace_id: "ws_13".into(),
        number: 13,
        label: "new-space".into(),
        ..template
    });
    update.focused_workspace_id = Some("ws_13".into());
    for workspace in &mut update.workspaces {
        workspace.focused = workspace.workspace_id == "ws_13";
    }
    state.set_snapshot(Box::new(update));
    let mut updated_surface = surface();
    updated_surface.projection_revision = 2;
    state.set_pane_surface(updated_surface);
    state.compose(106, 2).expect("zero-height workspace body");
    assert!(state.reveal_focused_workspace);
    state.compose(106, 20).expect("new workspace revealed");
    assert!(state
        .hits
        .workspaces
        .iter()
        .any(|hit| { hit.endpoint_id == ClientEndpointId::Local && hit.workspace_id == "ws_13" }));

    state.workspace_scroll = 0;
    state.compose(106, 20).expect("manual scroll");
    assert_eq!(state.workspace_scroll, 0);
    assert!(!state
        .hits
        .workspaces
        .iter()
        .any(|hit| { hit.endpoint_id == ClientEndpointId::Local && hit.workspace_id == "ws_13" }));
    let unchanged = state.snapshot.as_deref().expect("snapshot").clone();
    state.set_snapshot(Box::new(unchanged));
    state
        .compose(106, 20)
        .expect("unchanged focus preserves scroll");
    assert_eq!(state.workspace_scroll, 0);
}

#[test]
fn expanded_machine_sidebar_applies_space_row_gap_within_each_machine() {
    let (mut state, remote_id) = state_with_remote();
    state.config.spaces.row_gap = 1;

    let add_second_workspace = |snapshot: &mut ClientShellSnapshot| {
        let mut workspace = snapshot.workspaces[0].clone();
        workspace.workspace_id = "ws_2".into();
        workspace.active_tab_id = "tab_2".into();
        workspace.number = 2;
        workspace.label = "second-workspace".into();
        workspace.focused = false;
        snapshot.workspaces.push(workspace);
    };
    let mut local = snapshot();
    add_second_workspace(&mut local);
    state.set_snapshot(Box::new(local));
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.workspaces[0].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    add_second_workspace(&mut remote);
    remote.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    let mut third = remote.workspaces[1].clone();
    third.workspace_id = "ws_3".into();
    third.number = 3;
    third.label = "third-workspace".into();
    third.worktree = None;
    remote.workspaces.push(third);
    state.set_endpoint_snapshot(&remote_id, Box::new(remote));

    state.compose(100, 40).expect("combined endpoint frame");
    let local_workspaces = state
        .hits
        .workspaces
        .iter()
        .filter(|hit| hit.endpoint_id.is_local())
        .collect::<Vec<_>>();
    assert_eq!(local_workspaces.len(), 2);
    assert_eq!(
        local_workspaces[1].rect.y,
        local_workspaces[0].rect.bottom() + 1
    );

    let local_machine = state
        .hits
        .machines
        .iter()
        .find(|hit| hit.endpoint_id.is_local())
        .expect("local machine");
    let remote_machine = state
        .hits
        .machines
        .iter()
        .find(|hit| hit.endpoint_id == remote_id)
        .expect("remote machine");
    assert_eq!(local_workspaces[0].rect.y, local_machine.rect.bottom());
    assert_eq!(remote_machine.rect.y, local_workspaces[1].rect.bottom());

    let remote_workspaces = state
        .hits
        .workspaces
        .iter()
        .filter(|hit| hit.endpoint_id == remote_id)
        .collect::<Vec<_>>();
    assert_eq!(remote_workspaces.len(), 3);
    assert_eq!(remote_workspaces[0].rect.y, remote_machine.rect.bottom());
    assert_eq!(
        remote_workspaces[1].rect.y,
        remote_workspaces[0].rect.bottom()
    );
    assert_eq!(
        remote_workspaces[2].rect.y,
        remote_workspaces[1].rect.bottom() + 1
    );

    state.workspace_scroll = usize::MAX;
    state.compose(100, 18).expect("scrolled endpoint frame");
    let metrics = state
        .hits
        .workspace_scroll_metrics
        .expect("workspace scroll metrics");
    assert!(metrics.max_offset_from_bottom > 0);
    assert_eq!(metrics.offset_from_bottom, 0);
    assert_eq!(state.workspace_scroll, metrics.max_offset_from_bottom);
    let visible_remote = state
        .hits
        .workspaces
        .iter()
        .filter(|hit| hit.endpoint_id == remote_id)
        .collect::<Vec<_>>();
    assert_eq!(visible_remote.len(), 3);
    let gap_y = visible_remote[1].rect.bottom();
    assert_eq!(visible_remote[2].rect.y, gap_y + 1);
    assert!(visible_remote[2].rect.bottom() <= state.hits.workspace_body.bottom());
    assert!(state
        .hits
        .workspaces
        .iter()
        .all(|hit| gap_y < hit.rect.top() || gap_y >= hit.rect.bottom()));
}

#[test]
fn active_workspace_is_the_only_highlight_when_machine_is_expanded() {
    let (mut state, endpoint_id) = state_with_remote();
    assert!(state.activate_endpoint_projection(&endpoint_id));
    let mut remote_surface = surface();
    remote_surface.boot_id = "remote-boot".into();
    state.set_pane_surface(remote_surface);

    let frame = state.compose(100, 28).expect("combined endpoint frame");
    let machine = state
        .hits
        .machines
        .iter()
        .find(|hit| hit.endpoint_id == endpoint_id)
        .expect("remote machine hit")
        .rect;
    let workspace = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == endpoint_id)
        .expect("remote workspace hit")
        .rect;
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    assert_ne!(
        buffer[(machine.x, machine.y)].bg,
        state.config.palette.active_row_bg
    );
    assert_eq!(
        buffer[(workspace.x + 2, workspace.y)].bg,
        state.config.palette.active_row_bg
    );

    state.collapsed_endpoints.insert(endpoint_id.clone());
    let frame = state.compose(100, 28).expect("collapsed endpoint frame");
    let machine = state
        .hits
        .machines
        .iter()
        .find(|hit| hit.endpoint_id == endpoint_id)
        .expect("remote machine hit")
        .rect;
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    assert_eq!(
        buffer[(machine.x, machine.y)].bg,
        state.config.palette.active_row_bg
    );
}

#[test]
fn aggregate_agents_use_configured_rows_machine_token_and_status_colors() {
    use crate::api::schema::AgentStatus;
    use crate::config::{AgentSidebarToken, StatusIndicatorStyle};

    let mut config = Config::default();
    config.ui.status_indicators = StatusIndicatorStyle::Symbols;
    config.ui.sidebar.agents.rows = vec![vec![
        AgentSidebarToken::StateIcon,
        AgentSidebarToken::Machine,
        AgentSidebarToken::Agent,
    ]];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agents = vec![agent("local agent", AgentStatus::Idle, 1)];
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agents = vec![agent("remote agent", AgentStatus::Blocked, 1)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let frame = state.compose(100, 28).expect("combined endpoint frame");
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
    assert!(text.contains("○ Local · local agent"), "frame: {text}");
    assert!(text.contains("× Build · remote agent"), "frame: {text}");
    assert!(text.contains("grouped"), "frame: {text}");
    let toggle = state.hits.agent_sort_toggle;
    assert!(!toggle.is_empty());
    let click = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: toggle.x,
        row: toggle.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert_eq!(
        state.config.agent_panel_sort,
        crate::config::AgentPanelSortConfig::Priority
    );
    assert!(click.actions.is_empty());

    let buffer = frame
        .to_ratatui_buffer()
        .expect("aggregate frame should reconstruct");
    assert!(buffer
        .content()
        .iter()
        .any(|cell| cell.symbol() == "×" && cell.fg == state.config.palette.red));
}

#[test]
fn current_workspace_agent_view_excludes_same_workspace_id_on_other_machine() {
    use crate::api::schema::AgentStatus;
    use crate::config::AgentSidebarToken;

    let mut config = Config::default();
    config.ui.sidebar.agents.rows =
        vec![vec![AgentSidebarToken::Machine, AgentSidebarToken::Agent]];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agent_view_label = Some("current space".into());
    local.agent_order = vec!["pane_1".into()];
    local.agents = vec![agent("local agent", AgentStatus::Idle, 1)];
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());

    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agent_view_label = Some("current space".into());
    remote.agent_order = vec!["pane_1".into()];
    remote.agents = vec![agent("remote agent", AgentStatus::Idle, 1)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));
    let view = current_workspace_view();
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, Some(view.clone()));
    state.set_test_endpoint_agent_view(&endpoint_id, Some(view));

    let frame = state.compose(100, 28).expect("combined endpoint frame");
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
    assert!(text.contains("Local · local agent"), "frame: {text}");
    assert!(!text.contains("Build · remote agent"), "frame: {text}");

    assert!(state.activate_endpoint_projection(&endpoint_id));
    let mut remote_surface = surface();
    remote_surface.boot_id = "remote-boot".into();
    state.set_pane_surface(remote_surface);
    let frame = state.compose(100, 28).expect("remote endpoint frame");
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
    assert!(!text.contains("Local · local agent"), "frame: {text}");
    assert!(text.contains("Build · remote agent"), "frame: {text}");
}

#[test]
fn current_workspace_or_blocked_keeps_foreign_attention_only() {
    use crate::api::schema::{
        AgentStatus, AgentViewBuiltinField, AgentViewField, AgentViewFilter, AgentViewValue,
    };
    use crate::config::AgentSidebarToken;

    let mut config = Config::default();
    config.ui.sidebar.agents.rows =
        vec![vec![AgentSidebarToken::Machine, AgentSidebarToken::Agent]];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agent_view_label = Some("focus".into());
    local.agents = vec![agent("local agent", AgentStatus::Idle, 1)];
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());

    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agents = vec![
        agent("remote idle", AgentStatus::Idle, 1),
        ClientShellAgent {
            pane_id: "pane_2".into(),
            name: Some("remote blocked".into()),
            agent_status: AgentStatus::Blocked,
            focused: false,
            ..agent("remote blocked", AgentStatus::Blocked, 2)
        },
    ];
    remote.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        focused: false,
        ..remote.panes[0].clone()
    });
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let mut view = current_workspace_view();
    view.label = Some("focus".into());
    view.filter = Some(AgentViewFilter::Any {
        filters: vec![
            view.filter.take().expect("current workspace filter"),
            AgentViewFilter::Eq {
                field: AgentViewField::Builtin(AgentViewBuiltinField::Status),
                value: AgentViewValue::String("blocked".into()),
            },
        ],
    });
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, Some(view));

    let frame = state.compose(100, 28).expect("combined endpoint frame");
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
    assert!(text.contains("Local · local agent"), "frame: {text}");
    assert!(!text.contains("Build · remote idle"), "frame: {text}");
    assert!(text.contains("Build · remote blocked"), "frame: {text}");
}

#[test]
fn selected_default_view_ignores_inactive_endpoint_projection() {
    use crate::api::schema::{
        AgentStatus, AgentViewBuiltinField, AgentViewField, AgentViewFilter, AgentViewValue,
    };
    use crate::config::AgentSidebarToken;

    let mut config = Config::default();
    config.ui.sidebar.agents.rows =
        vec![vec![AgentSidebarToken::Machine, AgentSidebarToken::Agent]];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agents = vec![agent("local agent", AgentStatus::Idle, 1)];
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agent_view_label = Some("blocked".into());
    remote.agent_order.clear();
    remote.agents = vec![agent("remote agent", AgentStatus::Idle, 1)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, None);
    state.set_test_endpoint_agent_view(
        &endpoint_id,
        Some(crate::api::schema::AgentViewSetParams {
            source: "remote.views".into(),
            label: Some("blocked".into()),
            filter: Some(AgentViewFilter::Eq {
                field: AgentViewField::Builtin(AgentViewBuiltinField::Status),
                value: AgentViewValue::String("blocked".into()),
            }),
            sort: Vec::new(),
        }),
    );

    let frame = state.compose(100, 28).expect("combined endpoint frame");
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
    assert!(text.contains("Local · local agent"), "frame: {text}");
    assert!(text.contains("Build · remote agent"), "frame: {text}");
    assert!(text.contains("grouped"), "frame: {text}");
}

#[test]
fn newer_snapshot_does_not_reuse_stale_agent_view_projection() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut local = snapshot();
    local.agent_view_label = Some("current space".into());
    state.set_snapshot(Box::new(local.clone()));
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, Some(current_workspace_view()));
    let endpoint = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id.is_local())
        .expect("local endpoint");
    assert!(matches!(
        ClientShellState::endpoint_agent_view(endpoint),
        Some(Ok(Some(_)))
    ));

    state.set_test_endpoint_agent_view_projection(
        &ClientEndpointId::Local,
        "foreign-boot",
        99,
        None,
    );
    let endpoint = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id.is_local())
        .expect("local endpoint");
    assert!(matches!(
        ClientShellState::endpoint_agent_view(endpoint),
        Some(Ok(Some(_)))
    ));

    local.revision += 1;
    state.set_snapshot(Box::new(local));
    let endpoint = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id.is_local())
        .expect("local endpoint");
    assert!(ClientShellState::endpoint_agent_view(endpoint).is_none());
}

#[test]
fn legacy_custom_views_keep_v1_per_endpoint_projection() {
    use crate::api::schema::AgentStatus;
    use crate::config::AgentSidebarToken;

    let mut config = Config::default();
    config.ui.sidebar.agents.rows =
        vec![vec![AgentSidebarToken::Machine, AgentSidebarToken::Agent]];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agent_view_label = Some("current space".into());
    local.agent_order = vec!["pane_1".into()];
    local.agents = vec![agent("local agent", AgentStatus::Idle, 1)];
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agent_view_label = Some("current space".into());
    remote.agent_order = vec!["pane_1".into()];
    remote.agents = vec![agent("remote agent", AgentStatus::Idle, 1)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let frame = state.compose(100, 28).expect("legacy combined frame");
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
    assert!(text.contains("Local · local agent"), "frame: {text}");
    assert!(text.contains("Build · remote agent"), "frame: {text}");
}

#[test]
fn selected_custom_sort_orders_rendering_and_indexed_navigation() {
    use crate::api::schema::{
        AgentStatus, AgentViewBuiltinSortField, AgentViewSort, AgentViewSortField,
        AgentViewSortOrder,
    };
    use crate::config::AgentSidebarToken;

    let mut config = Config::default();
    config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    config.ui.sidebar.agents.rows =
        vec![vec![AgentSidebarToken::Machine, AgentSidebarToken::Agent]];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agent_view_label = Some("recent".into());
    local.agents = vec![agent("local blocked", AgentStatus::Blocked, 1)];
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agents = vec![agent("remote idle", AgentStatus::Idle, 9)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let mut view = current_workspace_view();
    view.label = Some("recent".into());
    view.filter = None;
    view.sort = vec![AgentViewSort {
        field: AgentViewSortField::Builtin(AgentViewBuiltinSortField::StateChangeSeq),
        order: AgentViewSortOrder::Desc,
    }];
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, Some(view));

    let frame = state.compose(100, 28).expect("custom sorted frame");
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
        text.find("Build · remote idle").expect("remote row")
            < text.find("Local · local blocked").expect("local row"),
        "frame: {text}"
    );

    let mut outcome = ClientShellInput::default();
    assert!(
        state.handle_endpoint_navigation(crate::input::KeybindAction::FocusAgent(0), &mut outcome,)
    );
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: selected,
            target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
        }] if selected == &endpoint_id && pane_id == "pane_1"
    ));
}

#[test]
fn selected_position_sort_uses_public_tab_and_pane_numbers() {
    use crate::api::schema::{
        AgentStatus, AgentViewBuiltinSortField, AgentViewSort, AgentViewSortField,
        AgentViewSortOrder,
    };

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut selected = snapshot();
    selected.agent_view_label = Some("positions".into());

    let mut tab_nine = selected.tabs[0].clone();
    tab_nine.tab_id = "ws_1:t9".into();
    tab_nine.number = 9;
    let mut tab_two = tab_nine.clone();
    tab_two.tab_id = "ws_1:t2".into();
    tab_two.number = 2;
    selected.tabs = vec![tab_nine, tab_two];

    let mut pane_tab_nine = selected.panes[0].clone();
    pane_tab_nine.tab_id = "ws_1:t9".into();
    pane_tab_nine.pane_id = "ws_1:p1".into();
    let mut pane_nine = pane_tab_nine.clone();
    pane_nine.tab_id = "ws_1:t2".into();
    pane_nine.pane_id = "ws_1:p9".into();
    let mut pane_two = pane_nine.clone();
    pane_two.pane_id = "ws_1:p2".into();
    selected.panes = vec![pane_tab_nine, pane_nine, pane_two];

    let mut late_tab = agent("tab nine", AgentStatus::Idle, 1);
    late_tab.tab_id = "ws_1:t9".into();
    late_tab.pane_id = "ws_1:p1".into();
    let mut late_pane = agent("pane nine", AgentStatus::Idle, 1);
    late_pane.tab_id = "ws_1:t2".into();
    late_pane.pane_id = "ws_1:p9".into();
    let mut early_pane = agent("pane two", AgentStatus::Idle, 1);
    early_pane.tab_id = "ws_1:t2".into();
    early_pane.pane_id = "ws_1:p2".into();
    selected.agents = vec![late_tab, late_pane, early_pane];
    state.set_snapshot(Box::new(selected));

    let mut view = current_workspace_view();
    view.label = Some("positions".into());
    view.filter = None;
    view.sort = vec![
        AgentViewSort {
            field: AgentViewSortField::Builtin(AgentViewBuiltinSortField::TabOrder),
            order: AgentViewSortOrder::Asc,
        },
        AgentViewSort {
            field: AgentViewSortField::Builtin(AgentViewBuiltinSortField::PaneOrder),
            order: AgentViewSortOrder::Asc,
        },
    ];
    state.set_test_endpoint_agent_view(&ClientEndpointId::Local, Some(view));

    let names = aggregate_navigation::aggregate_agent_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        crate::config::AgentPanelSortConfig::Priority,
    )
    .into_iter()
    .map(|row| row.agent.name.as_deref().expect("agent name"))
    .collect::<Vec<_>>();
    assert_eq!(names, ["pane two", "pane nine", "tab nine"]);
}

#[test]
fn aggregate_priority_uses_client_observed_recency_across_machines() {
    use crate::api::schema::AgentStatus;
    use crate::config::AgentSidebarToken;

    let mut config = Config::default();
    config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    config.ui.sidebar.agents.rows =
        vec![vec![AgentSidebarToken::Machine, AgentSidebarToken::Agent]];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let profile = remote_profile();
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agents = vec![agent("local agent", AgentStatus::Idle, 1)];
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agents = vec![agent("remote agent", AgentStatus::Idle, 1)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote.clone()));

    let mut local = snapshot();
    local.agents = vec![agent("local agent", AgentStatus::Idle, 2)];
    state.set_snapshot(Box::new(local));
    let frame_text = |state: &mut ClientShellState| {
        let frame = state.compose(100, 28).expect("combined endpoint frame");
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
    };
    let text = frame_text(&mut state);
    assert!(
        text.find("Local · local agent").expect("local agent")
            < text.find("Build · remote agent").expect("remote agent")
    );

    remote.agents = vec![agent("remote agent", AgentStatus::Working, 2)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote.clone()));
    let text = frame_text(&mut state);
    assert!(
        text.find("Build · remote agent").expect("remote agent")
            < text.find("Local · local agent").expect("local agent")
    );

    remote.agents = vec![agent("remote agent", AgentStatus::Idle, 3)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));
    let text = frame_text(&mut state);
    assert!(
        text.find("Build · remote agent").expect("remote agent")
            < text.find("Local · local agent").expect("local agent")
    );
    let mut outcome = ClientShellInput::default();
    assert!(
        state.handle_endpoint_navigation(crate::input::KeybindAction::FocusAgent(0), &mut outcome,)
    );
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
        }] if activated == &endpoint_id && pane_id == "pane_1"
    ));
}

#[test]
fn unselected_endpoint_completion_projects_done_client_side() {
    use crate::api::schema::AgentStatus;

    let (mut state, endpoint_id) = state_with_remote();
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agents = vec![agent("background agent", AgentStatus::Working, 2)];
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote.clone()));
    remote.revision = 2;
    remote.agents = vec![agent("background agent", AgentStatus::Idle, 3)];

    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));

    let status = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .and_then(|snapshot| snapshot.agents.first())
        .map(|agent| agent.agent_status);
    assert_eq!(status, Some(AgentStatus::Done));
    assert_eq!(state.active_endpoint_id, ClientEndpointId::Local);
}

#[test]
fn clicking_remote_machine_name_requests_activation_without_mutating_projection() {
    let (mut state, endpoint_id) = state_with_remote();
    state.compose(100, 28).expect("combined endpoint frame");
    let hit = state
        .hits
        .machines
        .iter()
        .find(|hit| hit.endpoint_id == endpoint_id)
        .expect("remote endpoint hit")
        .rect;
    let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.x + 3,
        row: hit.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: None,
        }] if activated == &endpoint_id
    ));
    assert_eq!(state.active_endpoint_id, ClientEndpointId::Local);
    assert_eq!(
        state
            .snapshot
            .as_deref()
            .map(|snapshot| snapshot.boot_id.as_str()),
        Some("boot-1")
    );
}

#[test]
fn clicking_local_can_cancel_a_remote_switch_while_local_is_still_displayed() {
    for workspace in [false, true] {
        let (mut state, remote) = state_with_remote();
        state.compose(100, 28).unwrap();
        let mut pending = ClientShellInput::default();
        assert!(state.activate_endpoint(remote, &mut pending));
        assert_eq!(state.active_endpoint_id, ClientEndpointId::Local);
        let rect = if workspace {
            state
                .hits
                .workspaces
                .iter()
                .find(|hit| hit.endpoint_id.is_local())
                .unwrap()
                .rect
        } else {
            state
                .hits
                .machines
                .iter()
                .find(|hit| hit.endpoint_id.is_local())
                .unwrap()
                .rect
        };
        let outcome = state.handle_raw_events(vec![
            RawInputEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x + 5,
                row: rect.y,
                modifiers: KeyModifiers::empty(),
            }),
            RawInputEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: rect.x + 5,
                row: rect.y,
                modifiers: KeyModifiers::empty(),
            }),
        ]);
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::ActivateEndpoint {
            endpoint_id: ClientEndpointId::Local, target,
        }] if target.is_some() == workspace)
        );
    }
}

#[test]
fn reconnecting_local_selection_still_reaches_the_runtime() {
    let (mut state, _) = state_with_remote();
    state.mark_endpoint_disconnected(&ClientEndpointId::Local);
    let mut outcome = ClientShellInput::default();
    state.focus_or_activate(
        ClientEndpointId::Local,
        ClientEndpointFocusTarget::Workspace("local-workspace".into()),
        &mut outcome,
    );
    assert!(
        matches!(outcome.actions.as_slice(), [ClientShellAction::ActivateEndpoint {
        endpoint_id: ClientEndpointId::Local,
        target: Some(ClientEndpointFocusTarget::Workspace(id)),
    }] if id == "local-workspace")
    );
}

#[test]
fn machine_arrow_toggles_inactive_machine_without_switching() {
    for sidebar_collapsed in [false, true] {
        for status in [
            ClientEndpointStatus::Online,
            ClientEndpointStatus::Reconnecting,
        ] {
            let (mut state, remote_id) = state_with_remote();
            let mut other_profile = remote_profile();
            other_profile.id = ProfileId::parse("1123456789abcdef0123456789abcdef").unwrap();
            let other_id = ClientEndpointId::Ssh(other_profile.id.clone());
            state.set_endpoint_catalog(&[remote_profile(), other_profile]);
            state.set_endpoint_status(&other_id, ClientEndpointStatus::Online);
            state.set_endpoint_snapshot(&other_id, Box::new(snapshot()));
            state.set_endpoint_status(&remote_id, status);
            state.sidebar_collapsed = sidebar_collapsed;

            for collapsed in [true, false] {
                let frame = state.compose(100, 28).expect("three machine frame");
                let machine = state
                    .hits
                    .machines
                    .iter()
                    .find(|hit| hit.endpoint_id == remote_id)
                    .expect("remote machine")
                    .rect;
                let column = machine.x + u16::from(!sidebar_collapsed);
                let buffer = frame.to_ratatui_buffer().expect("frame buffer");
                assert_eq!(
                    buffer[(column, machine.y)].symbol(),
                    if collapsed { "▾" } else { "▸" }
                );
                let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row: machine.y,
                    modifiers: KeyModifiers::empty(),
                })]);
                assert!(
                    outcome.actions.is_empty(),
                    "collapse must not switch machines"
                );
                assert!(outcome.requests.is_empty());
                assert!(outcome.repaint);
                assert_eq!(state.active_endpoint_id, ClientEndpointId::Local);
                assert_eq!(state.snapshot.as_ref().unwrap().boot_id, "boot-1");
                assert_eq!(
                    state
                        .snapshot
                        .as_ref()
                        .unwrap()
                        .focused_workspace_id
                        .as_deref(),
                    Some("ws_1")
                );
                assert_eq!(state.collapsed_endpoints.contains(&remote_id), collapsed);
                assert!(!state.collapsed_endpoints.contains(&ClientEndpointId::Local));
                assert!(!state.collapsed_endpoints.contains(&other_id));
                assert!(state.endpoint_error.is_none());

                state.compose(100, 28).expect("toggled machine frame");
                assert_eq!(
                    state
                        .hits
                        .workspaces
                        .iter()
                        .any(|hit| hit.endpoint_id == remote_id),
                    !collapsed
                );
                for endpoint_id in [&ClientEndpointId::Local, &other_id] {
                    assert!(state
                        .hits
                        .workspaces
                        .iter()
                        .any(|hit| &hit.endpoint_id == endpoint_id));
                }
            }
        }
    }
}

#[test]
fn context_menu_lookup_ignores_inactive_endpoint_workspaces() {
    let (mut state, endpoint_id) = state_with_remote();
    state.compose(100, 28).expect("combined endpoint frame");
    let remote = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == endpoint_id)
        .expect("remote workspace")
        .rect;
    let local = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id.is_local())
        .expect("local workspace")
        .rect;

    assert_eq!(
        state.active_endpoint_workspace_at((remote.x, remote.y)),
        None
    );
    assert_eq!(
        state.active_endpoint_workspace_at((local.x, local.y)),
        Some("ws_1".into())
    );
}

#[test]
fn future_surface_waits_for_its_exact_snapshot_revision() {
    let (mut state, _) = state_with_remote();
    let mut future = surface();
    future.projection_revision = 2;
    future.surface_revision = 2;
    state.set_pane_surface(future);
    assert_eq!(
        state
            .pane_surface
            .as_ref()
            .map(|surface| surface.projection_revision),
        Some(1)
    );
    assert_eq!(
        state
            .pending_pane_surface
            .as_ref()
            .map(|surface| surface.projection_revision),
        Some(2)
    );

    let mut next = snapshot();
    next.revision = 2;
    state.set_snapshot(Box::new(next));
    assert_eq!(
        state
            .pane_surface
            .as_ref()
            .map(|surface| surface.projection_revision),
        Some(2)
    );
    assert!(state.pending_pane_surface.is_none());
}

#[test]
fn inactive_endpoint_snapshot_cache_never_regresses_revision() {
    let (mut state, endpoint_id) = state_with_remote();
    let mut newest = snapshot();
    newest.boot_id = "remote-boot".into();
    newest.revision = 3;
    newest.workspaces[0].label = "newest".into();
    state.set_endpoint_snapshot(&endpoint_id, Box::new(newest));
    let mut delayed = snapshot();
    delayed.boot_id = "remote-boot".into();
    delayed.revision = 2;
    delayed.workspaces[0].label = "delayed".into();

    state.set_endpoint_snapshot(&endpoint_id, Box::new(delayed));

    let label = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .and_then(|snapshot| snapshot.workspaces.first())
        .map(|workspace| workspace.label.as_str());
    assert_eq!(label, Some("newest"));
}

#[test]
fn new_connection_generation_accepts_a_lower_same_boot_projection_revision() {
    let (mut state, endpoint_id) = state_with_remote();
    let mut previous = snapshot();
    previous.boot_id = "shared-server-boot".into();
    previous.revision = 9;
    previous.workspaces[0].label = "old connection".into();
    state.cache_endpoint_snapshot_inactive_for_generation(&endpoint_id, 4, Box::new(previous));
    let mut reconnected = snapshot();
    reconnected.boot_id = "shared-server-boot".into();
    reconnected.revision = 1;
    reconnected.workspaces[0].label = "new connection".into();

    state.cache_endpoint_snapshot_inactive_for_generation(&endpoint_id, 5, Box::new(reconnected));

    let endpoint = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("remote endpoint");
    assert_eq!(endpoint.snapshot_generation, Some(5));
    assert_eq!(endpoint.snapshot.as_ref().unwrap().revision, 1);
    assert_eq!(
        endpoint.snapshot.as_ref().unwrap().workspaces[0].label,
        "new connection"
    );
}

#[test]
fn reconnect_same_endpoint_accepts_new_generation_surface_revision() {
    for previous_revision in [9, 1] {
        let (mut state, endpoint_id) = state_with_remote();
        let mut previous = snapshot();
        previous.boot_id = "shared-server-boot".into();
        previous.revision = previous_revision;
        state.cache_endpoint_snapshot_inactive_for_generation(&endpoint_id, 4, Box::new(previous));
        assert!(state.activate_endpoint_projection(&endpoint_id));
        let mut previous_surface = surface();
        previous_surface.boot_id = "shared-server-boot".into();
        previous_surface.projection_revision = previous_revision;
        previous_surface.surface_revision = 9;
        state.set_pane_surface(previous_surface.clone());
        previous_surface.projection_revision += 1;
        state.set_pane_surface(previous_surface);
        assert!(state.pending_pane_surface.is_some());
        state.agent_scroll = 7;

        state.mark_endpoint_disconnected(&endpoint_id);
        let mut reconnected = snapshot();
        reconnected.boot_id = "shared-server-boot".into();
        reconnected.revision = 1;
        state.cache_endpoint_snapshot_inactive_for_generation(
            &endpoint_id,
            5,
            Box::new(reconnected),
        );
        assert_eq!(state.snapshot.as_ref().unwrap().revision, previous_revision);
        assert_eq!(state.pane_surface.as_ref().unwrap().surface_revision, 9);

        state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
        assert!(state.activate_endpoint_projection(&endpoint_id));
        assert!(state.compose(106, 20).is_none());
        let mut reconnected_surface = surface();
        reconnected_surface.boot_id = "shared-server-boot".into();
        reconnected_surface.projection_revision = 1;
        reconnected_surface.surface_revision = 1;
        state.set_pane_surface(reconnected_surface);

        assert_eq!(state.snapshot.as_ref().unwrap().revision, 1);
        assert_eq!(state.pane_surface.as_ref().unwrap().projection_revision, 1);
        assert_eq!(state.pane_surface.as_ref().unwrap().surface_revision, 1);
        assert!(state.pending_pane_surface.is_none());
        assert_eq!(state.agent_scroll, 7);
        assert!(state.compose(106, 20).is_some());
    }
}

#[test]
fn reconnect_snapshot_waits_for_coherent_activation_before_replacing_projection() {
    let (mut state, endpoint_id) = state_with_remote();
    assert!(state.activate_endpoint_projection(&endpoint_id));
    assert_eq!(state.snapshot.as_deref().unwrap().boot_id, "remote-boot");

    state.mark_endpoint_disconnected(&endpoint_id);
    let mut replacement = snapshot();
    replacement.boot_id = "replacement-boot".into();
    state.cache_endpoint_snapshot(&endpoint_id, Box::new(replacement));
    assert_eq!(state.snapshot.as_deref().unwrap().boot_id, "remote-boot");

    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
    assert!(state.activate_endpoint_projection(&endpoint_id));
    assert_eq!(
        state.snapshot.as_deref().unwrap().boot_id,
        "replacement-boot"
    );
}

#[test]
fn disconnected_active_endpoint_freezes_surface_and_marks_cached_ui_stale() {
    use crate::api::schema::AgentStatus;
    use crate::config::{AgentSidebarToken, StatusIndicatorStyle};

    let (mut state, endpoint_id) = state_with_remote();
    state.config.status_indicators = StatusIndicatorStyle::Symbols;
    state.config.agents.rows = vec![vec![
        AgentSidebarToken::StateIcon,
        AgentSidebarToken::Machine,
        AgentSidebarToken::Agent,
    ]];
    let endpoint = state
        .endpoints
        .iter_mut()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("remote endpoint");
    endpoint.snapshot.as_mut().expect("remote snapshot").agents =
        vec![agent("remote agent", AgentStatus::Blocked, 1)];
    assert!(state.activate_endpoint_projection(&endpoint_id));
    let mut remote_surface = surface();
    remote_surface.boot_id = "remote-boot".into();
    state.set_pane_surface(remote_surface);
    state.pending_integration_installs = 2;

    state.mark_endpoint_disconnected(&endpoint_id);
    let frame = state.compose(100, 28).expect("frozen endpoint frame");
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

    assert_eq!(state.pending_integration_installs, 0);
    assert_eq!(
        state.endpoint_status(&endpoint_id),
        Some(ClientEndpointStatus::Reconnecting)
    );
    assert!(text.contains("◐ reconnecting"), "frame: {text}");
    assert!(text.contains("Build · remote agent"), "frame: {text}");
    assert!(
        text.contains("LIVE"),
        "frozen surface should remain: {text}"
    );
    assert!(state.hits.panes.is_empty());
    assert!(frame.cursor.is_none());
    let buffer = frame.to_ratatui_buffer().expect("frame should reconstruct");
    let stale_icon = buffer
        .content()
        .iter()
        .find(|cell| cell.symbol() == "×" && cell.fg == state.config.palette.overlay0)
        .expect("stale blocked icon");
    assert_eq!(stale_icon.fg, state.config.palette.overlay0);
}

#[cfg(unix)]
#[test]
fn graphics_scope_qualifies_colliding_boot_ids_by_endpoint() {
    let (mut state, endpoint_id) = state_with_remote();
    let local_scope = state.graphics_scope().to_owned();
    let mut remote = snapshot();
    remote.boot_id = "boot-1".into();
    state.set_endpoint_snapshot(&endpoint_id, Box::new(remote));
    assert!(state.activate_endpoint_projection(&endpoint_id));
    let remote_scope = state.graphics_scope();
    assert_ne!(local_scope, remote_scope);
    assert!(remote_scope.starts_with("ssh:0123456789abcdef0123456789abcdef:"));
}

#[cfg(unix)]
#[test]
fn local_direct_graphics_accept_server_ids_across_endpoint_switches_and_restarts() {
    use crate::kitty_graphics::surface::{direct_upload_control, host_image_id};
    use crate::protocol::{SurfaceGraphicsAssetKey, SurfaceGraphicsFormat, SurfaceGraphicsSource};

    let (mut state, remote_id) = state_with_remote();
    let asset = SurfaceGraphicsAssetKey {
        source: SurfaceGraphicsSource::PaneLayer {
            pane_id: "pane_1".into(),
            layer_id: "primary".into(),
        },
        image_width: 2,
        image_height: 2,
        format: SurfaceGraphicsFormat::Rgba,
        data_len: 16,
        data_fingerprint: 17,
    };
    let (server_image_id, _) = direct_upload_control("boot-1", &asset);
    assert_eq!(
        host_image_id(state.graphics_scope(), &asset),
        server_image_id
    );
    assert!(state.trust_direct_graphics_asset(&asset, server_image_id));
    assert!(!state.trust_direct_graphics_asset(&asset, server_image_id + 1));

    let mut remote = snapshot();
    remote.boot_id = "boot-1".into();
    state.set_endpoint_snapshot(&remote_id, Box::new(remote));
    assert!(state.activate_endpoint_projection(&remote_id));
    assert!(!state.trust_direct_graphics_asset(&asset, server_image_id));
    assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
    assert!(state.trust_direct_graphics_asset(&asset, server_image_id));

    let mut restarted = snapshot();
    restarted.boot_id = "replacement-boot".into();
    state.set_snapshot(Box::new(restarted));
    let (restarted_image_id, _) = direct_upload_control("replacement-boot", &asset);
    assert!(!state.trust_direct_graphics_asset(&asset, server_image_id));
    assert_eq!(
        host_image_id(state.graphics_scope(), &asset),
        restarted_image_id
    );
    assert!(state.trust_direct_graphics_asset(&asset, restarted_image_id));
}

#[test]
fn navigator_uses_machine_parents_only_for_federated_clients() {
    let (mut state, _) = state_with_remote();
    state.open_navigator_overlay();
    let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
    else {
        panic!("expected navigator");
    };
    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
    let machines = rows
        .iter()
        .filter(|row| matches!(row.target, ClientNavigatorTarget::Machine { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        machines
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Local", "Build"]
    );
    assert!(rows.iter().all(|row| {
        matches!(row.target, ClientNavigatorTarget::Machine { .. })
            || (!row.label.contains("Local ·") && !row.label.contains("Build ·"))
    }));
    assert!(rows.iter().all(|row| match row.target {
        ClientNavigatorTarget::Machine { .. } => row.depth == 0 && row.status.is_none(),
        ClientNavigatorTarget::Workspace { .. } => row.depth == 1 && row.status.is_none(),
        ClientNavigatorTarget::Pane { .. } => row.depth == 2 && row.status.is_some(),
    }));
    assert_eq!(rows.iter().filter(|row| row.current).count(), 1);

    let frame = state.compose(106, 30).expect("federated navigator");
    for (rect, target) in &state.hits.navigator_rows {
        let expected = match target {
            ClientNavigatorTarget::Machine { .. } => " ",
            ClientNavigatorTarget::Workspace { .. } => "   ",
            ClientNavigatorTarget::Pane { .. } => "   └─ ",
        };
        let prefix = frame.cells[rect.y as usize * frame.width as usize + rect.x as usize..]
            .iter()
            .take(expected.chars().count())
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert_eq!(prefix, expected, "{target:?}");
    }

    let mut local = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    local.set_snapshot(Box::new(snapshot()));
    local.set_pane_surface(surface());
    let frame = local.compose(100, 28).expect("local-only sidebar");
    assert!(local.hits.machines.is_empty());
    assert!(!frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .contains(" machines"));
    local.open_navigator_overlay();
    let ClientShellOverlay::Navigator(navigator) = local.overlay.as_ref().expect("navigator")
    else {
        panic!("expected navigator");
    };
    let rows =
        render::client_navigator_rows(&local.endpoints, &local.active_endpoint_id, navigator);
    assert!(rows
        .iter()
        .all(|row| !matches!(row.target, ClientNavigatorTarget::Machine { .. })));
    assert!(rows.iter().all(|row| match row.target {
        ClientNavigatorTarget::Workspace { .. } => row.depth == 0,
        ClientNavigatorTarget::Pane { .. } => row.depth == 1,
        ClientNavigatorTarget::Machine { .. } => false,
    }));
}

#[test]
fn navigator_keeps_saved_machine_visible_before_metadata_arrives() {
    let (mut state, endpoint_id) = state_with_remote();
    let endpoint = state
        .endpoints
        .iter_mut()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("saved remote endpoint");
    endpoint.snapshot = None;
    endpoint.status = ClientEndpointStatus::Connecting;
    state.open_navigator_overlay();
    let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
    else {
        panic!("expected navigator");
    };

    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);

    assert!(rows.iter().any(|row| {
        matches!(
            &row.target,
            ClientNavigatorTarget::Machine { endpoint_id: target } if target == &endpoint_id
        ) && row.label == "Build"
            && row.stale
    }));
    assert!(!rows.iter().any(|row| match &row.target {
        ClientNavigatorTarget::Machine { .. } => false,
        ClientNavigatorTarget::Workspace {
            endpoint_id: target,
            ..
        }
        | ClientNavigatorTarget::Pane {
            endpoint_id: target,
            ..
        } => target == &endpoint_id,
    }));
}

#[test]
fn navigator_machine_selection_opens_its_remembered_view() {
    let (mut state, endpoint_id) = state_with_remote();
    state.open_navigator_overlay();
    let selected = {
        let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
        else {
            panic!("expected navigator");
        };
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator)
            .into_iter()
            .find(|row| {
                matches!(
                    &row.target,
                    ClientNavigatorTarget::Machine { endpoint_id: target } if target == &endpoint_id
                )
            })
            .map(|row| row.target)
            .expect("remote machine row")
    };
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(selected);
    }

    let mut outcome = ClientShellInput::default();
    state.accept_navigator_selection(&mut outcome);

    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: None,
        }] if activated == &endpoint_id
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn navigator_foreign_pane_selection_activates_its_endpoint() {
    let (mut state, endpoint_id) = state_with_remote();
    state.open_navigator_overlay();
    let selected = {
        let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
        else {
            panic!("expected navigator");
        };
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator)
            .iter()
            .find(|row| {
                matches!(
                    &row.target,
                    ClientNavigatorTarget::Pane {
                        endpoint_id: target_endpoint,
                        pane_id,
                    } if target_endpoint == &endpoint_id && pane_id == "pane_1"
                )
            })
            .map(|row| row.target.clone())
            .expect("remote pane row")
    };
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(selected);
    }
    let mut local = snapshot();
    let mut inserted = local.workspaces[0].clone();
    inserted.workspace_id = "ws_2".into();
    inserted.focused = false;
    local.workspaces.push(inserted);
    state.set_snapshot(Box::new(local));

    let mut outcome = ClientShellInput::default();
    state.accept_navigator_selection(&mut outcome);

    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
        }] if activated == &endpoint_id && pane_id == "pane_1"
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn mobile_foreign_agent_and_workspace_targets_activate_their_endpoint() {
    use crate::api::schema::AgentStatus;

    let (mut state, endpoint_id) = state_with_remote();
    state
        .endpoints
        .iter_mut()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("remote endpoint")
        .snapshot
        .as_mut()
        .expect("remote snapshot")
        .agents = vec![agent("remote agent", AgentStatus::Working, 2)];
    state.mode = ClientShellMode::Navigate;
    state.compose(44, 30).expect("mobile switcher");
    let remote_agent = state
        .hits
        .mobile_targets
        .iter()
        .find_map(|(rect, target)| {
            matches!(
                target,
                ClientMobileTarget::Agent {
                    endpoint_id: target_endpoint,
                    pane_id,
                } if target_endpoint == &endpoint_id && pane_id == "pane_1"
            )
            .then_some(*rect)
        })
        .expect("remote agent target");
    let click = |rect: Rect| {
        RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })
    };
    let outcome = state.handle_raw_events(vec![click(remote_agent)]);
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
        }] if activated == &endpoint_id && pane_id == "pane_1"
    ));

    state.mode = ClientShellMode::Navigate;
    state.compose(44, 30).expect("mobile switcher");
    let remote_workspace = state
        .hits
        .mobile_targets
        .iter()
        .find_map(|(rect, target)| {
            matches!(
                target,
                ClientMobileTarget::Workspace {
                    endpoint_id: target_endpoint,
                    workspace_id,
                } if target_endpoint == &endpoint_id && workspace_id == "ws_1"
            )
            .then_some(*rect)
        })
        .expect("remote workspace target");
    let outcome = state.handle_raw_events(vec![click(remote_workspace)]);
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: Some(ClientEndpointFocusTarget::Workspace(workspace_id)),
        }] if activated == &endpoint_id && workspace_id == "ws_1"
    ));
}

#[test]
fn cached_offline_navigator_and_mobile_targets_are_dimmed_and_disabled() {
    use crate::api::schema::AgentStatus;

    let (mut state, endpoint_id) = state_with_remote();
    state
        .endpoints
        .iter_mut()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("remote endpoint")
        .snapshot
        .as_mut()
        .expect("remote snapshot")
        .agents = vec![agent("remote agent", AgentStatus::Blocked, 2)];
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Reconnecting);

    state.open_navigator_overlay();
    let selected = {
        let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
        else {
            panic!("expected navigator");
        };
        let rows =
            render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
        let machine = rows
            .iter()
            .find(|row| {
                matches!(
                    &row.target,
                    ClientNavigatorTarget::Machine { endpoint_id: target } if target == &endpoint_id
                )
            })
            .expect("cached remote machine row");
        assert!(machine.stale);
        let row = rows
            .iter()
            .find(|row| {
                matches!(
                    &row.target,
                    ClientNavigatorTarget::Pane {
                        endpoint_id: target_endpoint,
                        pane_id,
                    } if target_endpoint == &endpoint_id && pane_id == "pane_1"
                )
            })
            .expect("cached remote pane row");
        assert!(row.stale);
        assert!(!row.meta.contains("reconnecting"));
        row.target.clone()
    };
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(selected);
    }
    let mut navigator_outcome = ClientShellInput::default();
    state.accept_navigator_selection(&mut navigator_outcome);
    assert!(navigator_outcome.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));

    state.overlay = None;
    state.mode = ClientShellMode::Navigate;
    let frame = state.compose(44, 30).expect("offline mobile switcher");
    let mobile_target = state
        .hits
        .mobile_targets
        .iter()
        .find_map(|(rect, target)| {
            matches!(
                target,
                ClientMobileTarget::Workspace {
                    endpoint_id: target_endpoint,
                    workspace_id,
                } if target_endpoint == &endpoint_id && workspace_id == "ws_1"
            )
            .then_some(*rect)
        })
        .expect("cached remote workspace target");
    let buffer = frame.to_ratatui_buffer().expect("mobile frame buffer");
    assert_eq!(
        buffer[(mobile_target.x, mobile_target.y)].fg,
        state.config.palette.overlay0
    );
    let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: mobile_target.x,
        row: mobile_target.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(outcome.actions.is_empty());
    assert_eq!(state.mode, ClientShellMode::Navigate);
    assert_eq!(state.active_endpoint_id, ClientEndpointId::Local);
}

#[test]
fn focus_agent_index_uses_online_aggregate_rows() {
    use crate::api::schema::AgentStatus;

    let (mut state, endpoint_id) = state_with_remote();
    state
        .endpoints
        .iter_mut()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("remote endpoint")
        .snapshot
        .as_mut()
        .expect("remote snapshot")
        .agents = vec![agent("remote agent", AgentStatus::Working, 2)];
    let focus_agent =
        |index| crate::input::KeybindMatch::Action(crate::input::KeybindAction::FocusAgent(index));

    assert!(state.indexed_navigation_target_exists(&focus_agent(0)));
    assert!(!state.indexed_navigation_target_exists(&focus_agent(1)));

    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Reconnecting);
    assert!(!state.indexed_navigation_target_exists(&focus_agent(0)));
}

#[test]
fn workspace_drag_rejects_foreign_endpoint_slots() {
    let (mut state, endpoint_id) = state_with_remote();
    state.compose(100, 28).expect("aggregate sidebar");
    let local = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id.is_local())
        .expect("local workspace")
        .rect;
    let remote = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == endpoint_id)
        .expect("remote workspace")
        .rect;
    let mouse = |kind, rect: Rect| {
        RawInputEvent::Mouse(MouseEvent {
            kind,
            column: rect.x.saturating_add(1),
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })
    };

    state.handle_raw_events(vec![mouse(MouseEventKind::Down(MouseButton::Left), local)]);
    state.handle_raw_events(vec![mouse(MouseEventKind::Drag(MouseButton::Left), remote)]);

    assert!(state.chrome_drag.is_none());
}

#[test]
fn collapsed_aggregate_workspace_status_uses_its_status_color() {
    use crate::api::schema::AgentStatus;

    let (mut state, endpoint_id) = state_with_remote();
    state
        .endpoints
        .iter_mut()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("remote endpoint")
        .snapshot
        .as_mut()
        .expect("remote snapshot")
        .workspaces[0]
        .agent_status = AgentStatus::Blocked;
    state.sidebar_collapsed = true;

    let frame = state.compose(100, 28).expect("collapsed aggregate sidebar");
    let workspace = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.endpoint_id == endpoint_id)
        .expect("remote workspace")
        .rect;
    let buffer = frame.to_ratatui_buffer().expect("frame buffer");
    assert_eq!(
        buffer[(workspace.x.saturating_add(2), workspace.y)].fg,
        state.config.palette.red
    );
}

#[test]
fn navigator_workspace_arrows_cross_machine_headings_without_activating_them() {
    let (mut state, endpoint_id) = state_with_remote();
    state.open_navigator_overlay();
    for (key, expected_endpoint) in [
        (KeyCode::Right, endpoint_id),
        (KeyCode::Left, ClientEndpointId::Local),
    ] {
        let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(key, KeyModifiers::empty()),
        )]);
        assert!(outcome.actions.is_empty());
        let Some(ClientShellOverlay::Navigator(navigator)) = &state.overlay else {
            panic!("navigator");
        };
        assert_eq!(
            navigator.selected,
            Some(ClientNavigatorTarget::Pane {
                endpoint_id: expected_endpoint,
                pane_id: "pane_1".into(),
            })
        );
    }
}

#[test]
fn navigator_foreign_workspace_heading_keeps_the_workspace_target() {
    let (mut state, endpoint_id) = state_with_remote();
    state.open_navigator_overlay();
    let selected = {
        let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
        else {
            panic!("expected navigator");
        };
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator)
            .iter()
            .find(|row| {
                matches!(
                    &row.target,
                    ClientNavigatorTarget::Workspace {
                        endpoint_id: target_endpoint,
                        workspace_id,
                    } if target_endpoint == &endpoint_id && workspace_id == "ws_1"
                )
            })
            .map(|row| row.target.clone())
            .expect("remote workspace heading")
    };
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(selected);
    }

    let mut outcome = ClientShellInput::default();
    state.accept_navigator_selection(&mut outcome);

    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: Some(ClientEndpointFocusTarget::Workspace(workspace_id)),
        }] if activated == &endpoint_id && workspace_id == "ws_1"
    ));
}

fn navigator_multi_workspace_state(start_expanded: bool) -> ClientShellState {
    navigator_workspace_state(2, start_expanded)
}

/// `workspaces` workspaces, each with exactly one tab and one pane.
fn navigator_workspace_state(workspaces: usize, start_expanded: bool) -> ClientShellState {
    let mut config = Config::default();
    config.ui.navigator_start_expanded = start_expanded;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut projected = snapshot();
    projected.workspaces = (1..=workspaces)
        .map(|number| {
            let mut workspace = projected.workspaces[0].clone();
            workspace.workspace_id = format!("ws_{number}");
            workspace.label = format!("space-{number}");
            workspace.number = number;
            workspace.focused = number == 1;
            workspace
        })
        .collect();
    // Give each workspace its own tab and pane so both can be observed.
    let base_tab = projected.tabs[0].clone();
    let base_pane = projected.panes[0].clone();
    projected.tabs = (1..=workspaces)
        .map(|number| {
            let mut tab = base_tab.clone();
            tab.tab_id = format!("tab_{number}");
            tab.workspace_id = format!("ws_{number}");
            tab.number = number;
            tab.label = number.to_string();
            tab.focused = number == 1;
            tab
        })
        .collect();
    projected.panes = (1..=workspaces)
        .map(|number| {
            let mut pane = base_pane.clone();
            pane.pane_id = format!("pane_{number}");
            pane.workspace_id = format!("ws_{number}");
            pane.tab_id = format!("tab_{number}");
            pane.label = Some(format!("term-{number}"));
            pane.focused = number == 1;
            pane
        })
        .collect();
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state
}

fn navigator_workspace_keys(state: &ClientShellState) -> Vec<String> {
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should be open");
    };
    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
    rows.iter()
        .filter_map(|row| match &row.target {
            ClientNavigatorTarget::Workspace { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .collect()
}

fn navigator_pane_keys(state: &ClientShellState) -> Vec<String> {
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should be open");
    };
    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
    rows.iter()
        .filter_map(|row| match &row.target {
            ClientNavigatorTarget::Pane { pane_id, .. } => Some(pane_id.clone()),
            _ => None,
        })
        .collect()
}

/// The active endpoint's workspace keys currently listed in the collapse set.
/// Workspaces are expanded unless listed, so membership means "collapsed".
fn navigator_collapsed_workspace_keys(state: &ClientShellState) -> Vec<String> {
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should be open");
    };
    let mut keys = navigator
        .collapsed_workspaces
        .iter()
        .filter(|(endpoint_id, _)| *endpoint_id == state.active_endpoint_id)
        .map(|(_, workspace_id)| workspace_id.clone())
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

fn press_navigator_key(state: &mut ClientShellState, code: KeyCode) -> ClientShellInput {
    press_navigator_mod(state, code, KeyModifiers::empty())
}

fn press_navigator_mod(
    state: &mut ClientShellState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        code, modifiers,
    ))])
}

fn select_navigator_workspace(state: &mut ClientShellState, workspace_id: &str) {
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: workspace_id.to_owned(),
        });
    }
}

#[test]
fn navigator_start_expanded_shows_every_pane() {
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    assert_eq!(navigator_workspace_keys(&state), vec!["ws_1", "ws_2"]);
    assert_eq!(navigator_pane_keys(&state), vec!["pane_1", "pane_2"]);
}

#[test]
fn navigator_start_collapsed_hides_panes_behind_workspace_rows() {
    let mut state = navigator_multi_workspace_state(false);
    state.open_navigator_overlay();
    assert_eq!(navigator_workspace_keys(&state), vec!["ws_1", "ws_2"]);
    assert!(
        navigator_pane_keys(&state).is_empty(),
        "a collapsed workspace must not list its panes"
    );
}

#[test]
fn space_toggles_the_selected_workspace_collapse() {
    let mut state = navigator_multi_workspace_state(false);
    state.open_navigator_overlay();
    select_navigator_workspace(&mut state, "ws_1");
    press_navigator_key(&mut state, KeyCode::Char(' '));
    assert_eq!(
        navigator_pane_keys(&state),
        vec!["pane_1"],
        "expanding ws_1 should reveal only its own pane"
    );
    press_navigator_key(&mut state, KeyCode::Char(' '));
    assert!(navigator_pane_keys(&state).is_empty());
}

#[test]
fn collapsing_keeps_the_selection_on_the_workspace_row() {
    let mut state = navigator_multi_workspace_state(false);
    state.open_navigator_overlay();
    select_navigator_workspace(&mut state, "ws_2");
    press_navigator_key(&mut state, KeyCode::Char(' '));
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_2".into(),
        }),
        "toggling must not drop the highlight to the top of the list"
    );
    press_navigator_key(&mut state, KeyCode::Char(' '));
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_2".into(),
        }),
        "expanding again must keep the selection too"
    );
}

#[test]
fn navigator_search_cannot_match_a_pane_hidden_by_collapse() {
    let mut state = navigator_multi_workspace_state(false);
    state.open_navigator_overlay();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        // The workspace label matches, but its pane is hidden.
        navigator.query = "space-1".into();
    }
    assert_eq!(navigator_workspace_keys(&state), vec!["ws_1"]);
    assert!(
        navigator_pane_keys(&state).is_empty(),
        "a collapsed workspace's pane must not be reachable by search"
    );
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.query = "term-1".into();
    }
    assert!(
        navigator_workspace_keys(&state).is_empty(),
        "a query naming only a hidden pane matches nothing"
    );
    assert!(navigator_pane_keys(&state).is_empty());
}

#[test]
fn expanding_a_workspace_makes_its_pane_searchable_again() {
    let mut state = navigator_multi_workspace_state(false);
    state.open_navigator_overlay();
    select_navigator_workspace(&mut state, "ws_1");
    press_navigator_key(&mut state, KeyCode::Char(' '));
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.query = "term-1".into();
    }
    assert_eq!(navigator_pane_keys(&state), vec!["pane_1"]);
}

#[test]
fn space_on_a_pane_row_collapses_the_parent_workspace() {
    let mut state = navigator_multi_workspace_state(false);
    state.open_navigator_overlay();
    select_navigator_workspace(&mut state, "ws_1");
    press_navigator_key(&mut state, KeyCode::Char(' '));
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Pane {
            endpoint_id: state.active_endpoint_id.clone(),
            pane_id: "pane_1".into(),
        });
    }
    press_navigator_key(&mut state, KeyCode::Char(' '));
    assert!(
        navigator_pane_keys(&state).is_empty(),
        "space on a pane row collapses the workspace it is nested under"
    );
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_1".into(),
        }),
        "the highlight must land on the parent, not the next pane row"
    );
}

#[test]
fn navigator_collapse_all_workspaces_with_c() {
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    assert_eq!(
        navigator_collapsed_workspace_keys(&state),
        Vec::<String>::new(),
        "the popup starts fully expanded"
    );
    assert_eq!(navigator_pane_keys(&state), vec!["pane_1", "pane_2"]);

    // Select a pane row so the parent lookup is exercised.
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Pane {
            endpoint_id: state.active_endpoint_id.clone(),
            pane_id: "pane_2".into(),
        });
    }
    press_navigator_key(&mut state, KeyCode::Char('c'));

    assert_eq!(
        navigator_collapsed_workspace_keys(&state),
        vec!["ws_1", "ws_2"],
        "c lists every workspace in the collapse set"
    );
    assert_eq!(navigator_workspace_keys(&state), vec!["ws_1", "ws_2"]);
    assert!(navigator_pane_keys(&state).is_empty());
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_2".into(),
        }),
        "c keeps the highlight on the workspace the selected row belonged to"
    );
    assert_eq!(navigator.scroll, 0);
}

#[test]
fn navigator_expand_all_workspaces_with_e() {
    let mut state = navigator_multi_workspace_state(false);
    state.open_navigator_overlay();
    assert_eq!(
        navigator_collapsed_workspace_keys(&state),
        vec!["ws_1", "ws_2"],
        "the popup starts fully collapsed"
    );
    assert!(navigator_pane_keys(&state).is_empty());

    press_navigator_key(&mut state, KeyCode::Char('e'));

    assert!(
        navigator_collapsed_workspace_keys(&state).is_empty(),
        "e empties the collapse set"
    );
    assert_eq!(navigator_workspace_keys(&state), vec!["ws_1", "ws_2"]);
    assert_eq!(navigator_pane_keys(&state), vec!["pane_1", "pane_2"]);
}

#[test]
fn navigator_space_keeps_selection_and_collapses_to_parent() {
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    let workspace_target = |state: &ClientShellState| {
        let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
        else {
            panic!("expected navigator");
        };
        let rows =
            render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
        aggregate_navigation::selected_navigator_target(&rows, navigator)
    };
    let ws_1 = ClientNavigatorTarget::Workspace {
        endpoint_id: state.active_endpoint_id.clone(),
        workspace_id: "ws_1".into(),
    };
    // The popup opens on the focused pane; space collapses its parent workspace.
    press_navigator_key(&mut state, KeyCode::Char(' '));
    assert_eq!(workspace_target(&state), Some(ws_1.clone()));
    assert_eq!(
        navigator_workspace_keys(&state),
        vec!["ws_1", "ws_2"],
        "workspace rows stay visible when collapsed"
    );
    assert_eq!(
        navigator_pane_keys(&state),
        vec!["pane_2"],
        "only ws_1's pane is hidden"
    );
    // Space expands again without moving the selection off the workspace row.
    press_navigator_key(&mut state, KeyCode::Char(' '));
    assert_eq!(workspace_target(&state), Some(ws_1.clone()));
    assert_eq!(navigator_pane_keys(&state), vec!["pane_1", "pane_2"]);
    // Space on a child row collapses the parent workspace.
    press_navigator_key(&mut state, KeyCode::Char('j'));
    press_navigator_key(&mut state, KeyCode::Char(' '));
    assert_eq!(workspace_target(&state), Some(ws_1));
    assert_eq!(navigator_pane_keys(&state), vec!["pane_2"]);
}

#[test]
fn navigator_toggle_resets_scroll_to_the_workspace_row() {
    let mut state = navigator_workspace_state(40, true);
    state.open_navigator_overlay();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Pane {
            endpoint_id: state.active_endpoint_id.clone(),
            pane_id: "pane_30".into(),
        });
        navigator.scroll = 20;
    }
    press_navigator_key(&mut state, KeyCode::Char(' '));
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_30".into(),
        })
    );
    assert_eq!(navigator.scroll, 0, "the viewport returns to the top");
}

#[test]
fn navigator_footer_documents_the_browse_mode_keys() {
    // Pinned because the footer copy regressed to upstream text once already.
    const HINT: &str = " move j/k/ctrl+n/p · toggle space · expand/collapse e/c · filter F/b/w/i/d · search / · open enter · close x · back esc";

    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    let frame = state.compose(200, 40).expect("navigator frame");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| row.iter().map(|c| c.symbol.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    let footer = text
        .lines()
        .find(|line| line.contains("toggle space"))
        .expect("navigator footer row");

    // The hint is wider than the compact popup interior, so only the leading
    // part is visible. Compare against what actually fits rather than the whole
    // string, and pin the characters that reach the user.
    let interior = (state.hits.navigator_popup.width - 2) as usize;
    let visible = HINT.chars().take(interior).collect::<String>();
    assert!(
        footer.contains(visible.trim_end()),
        "footer row should show the leading hint, got {footer:?}"
    );
}

/// Renders the navigator and returns each row's rendered text with its target,
/// so tests can assert on the gutter decoration without recomputing row geometry.
fn navigator_rows_by_target(state: &mut ClientShellState) -> Vec<(ClientNavigatorTarget, String)> {
    let frame = state.compose(120, 34).expect("navigator frame");
    let buffer = frame.to_ratatui_buffer().expect("frame buffer");
    state
        .hits
        .navigator_rows
        .iter()
        .map(|(rect, target)| {
            let text = (rect.x..rect.right())
                .map(|x| buffer[(x, rect.y)].symbol())
                .collect::<String>();
            (target.clone(), text)
        })
        .collect()
}

fn navigator_row_text<'a>(
    rows: &'a [(ClientNavigatorTarget, String)],
    target: &ClientNavigatorTarget,
) -> &'a str {
    rows.iter()
        .find(|(row_target, _)| row_target == target)
        .map(|(_, text)| text.as_str())
        .unwrap_or_else(|| panic!("navigator row for {target:?}"))
}

fn navigator_workspace_target(
    state: &ClientShellState,
    workspace_id: &str,
) -> ClientNavigatorTarget {
    ClientNavigatorTarget::Workspace {
        endpoint_id: state.active_endpoint_id.clone(),
        workspace_id: workspace_id.into(),
    }
}

#[test]
fn navigator_workspace_rows_carry_a_collapse_caret() {
    // The caret was dropped when upstream rewrote the picker to list agents and
    // terminals, even though collapse/expand on space and e/c stayed. Asserted so
    // the marker cannot silently disappear again.
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();

    let rows = navigator_rows_by_target(&mut state);
    for workspace_id in ["ws_1", "ws_2"] {
        let target = navigator_workspace_target(&state, workspace_id);
        let text = navigator_row_text(&rows, &target);
        assert!(
            text.starts_with(" \u{25be}"),
            "expanded {workspace_id} should start with a down caret, got {text:?}"
        );
    }

    // Pane rows are not collapsible and must not claim the marker slot.
    let pane = ClientNavigatorTarget::Pane {
        endpoint_id: state.active_endpoint_id.clone(),
        pane_id: "pane_1".into(),
    };
    let pane_text = navigator_row_text(&rows, &pane);
    assert!(
        !pane_text.contains('\u{25be}') && !pane_text.contains('\u{25b8}'),
        "pane row should have no caret, got {pane_text:?}"
    );

    // Collapsing a workspace flips its caret and drops its pane rows.
    press_navigator_key(&mut state, KeyCode::Char(' '));
    let rows = navigator_rows_by_target(&mut state);
    let text = navigator_row_text(&rows, &navigator_workspace_target(&state, "ws_1"));
    assert!(
        text.starts_with(" \u{25b8}"),
        "collapsed ws_1 should start with a right caret, got {text:?}"
    );
    assert!(
        !rows.iter().any(|(target, _)| *target == pane),
        "collapsing ws_1 should hide its pane rows, rows: {rows:?}"
    );
}

#[test]
fn navigator_current_row_keeps_the_diamond_over_the_caret() {
    // The focused pane claims the diamond, so its workspace row still shows the
    // caret. Collapse both workspaces and select one of them: with no visible
    // pane left to claim the marker, the workspace row must show the diamond.
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    press_navigator_key(&mut state, KeyCode::Char('e'));

    let rows = navigator_rows_by_target(&mut state);
    let marked = rows
        .iter()
        .filter(|(_, text)| text.contains('\u{25c6}'))
        .count();
    assert_eq!(
        marked, 1,
        "exactly one row keeps the diamond, rows: {rows:?}"
    );

    // Select the second workspace so the marker moves with the focus.
    press_navigator_key(&mut state, KeyCode::Down);
    let mut outcome = ClientShellInput::default();
    state.accept_navigator_selection(&mut outcome);

    state.open_navigator_overlay();
    press_navigator_key(&mut state, KeyCode::Char('e'));
    let rows = navigator_rows_by_target(&mut state);
    let marked = rows
        .iter()
        .filter(|(_, text)| text.contains('\u{25c6}'))
        .count();
    assert_eq!(
        marked, 1,
        "the diamond must move to the newly focused workspace, rows: {rows:?}"
    );
}

#[test]
fn navigator_orders_worktree_groups_like_spaces_panel() {
    let mut projected = snapshot();
    let mut parent = projected.workspaces[0].clone();
    parent.workspace_id = "ws_1".into();
    parent.label = "va-web".into();
    parent.number = 1;
    parent.focused = false;
    parent.worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    let mut standalone = parent.clone();
    standalone.workspace_id = "ws_2".into();
    standalone.label = "neovim".into();
    standalone.number = 2;
    standalone.active_tab_id = "tab_ws_2".into();
    standalone.worktree = None;
    standalone.focused = false;
    let mut child = parent.clone();
    child.workspace_id = "ws_3".into();
    child.label = "feat".into();
    child.number = 3;
    child.active_tab_id = "tab_ws_3".into();
    child.focused = false;
    child.worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    // Raw creation order: child was created after the unrelated workspace.
    projected.workspaces = vec![parent, standalone, child];

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();
    assert_eq!(
        navigator_workspace_keys(&state),
        vec!["ws_1", "ws_3", "ws_2"],
        "the grouped parent and its child lead, standalone workspaces follow"
    );
}

fn two_workspace_focused_on_second_snapshot() -> ClientShellSnapshot {
    let mut snap = snapshot();
    let mut second_workspace = snap.workspaces[0].clone();
    second_workspace.workspace_id = "ws_2".into();
    second_workspace.active_tab_id = "tab_2".into();
    second_workspace.number = 2;
    second_workspace.label = "second-workspace".into();
    second_workspace.focused = true;
    snap.workspaces[0].focused = false;
    snap.workspaces.push(second_workspace);

    let mut second_tab = snap.tabs[0].clone();
    second_tab.tab_id = "tab_2".into();
    second_tab.workspace_id = "ws_2".into();
    second_tab.number = 2;
    second_tab.label = "2".into();
    second_tab.focused = true;
    snap.tabs[0].focused = false;
    snap.tabs.push(second_tab);

    let mut second_pane = snap.panes[0].clone();
    second_pane.pane_id = "pane_2".into();
    second_pane.workspace_id = "ws_2".into();
    second_pane.tab_id = "tab_2".into();
    second_pane.focused = true;
    snap.panes[0].focused = false;
    snap.panes.push(second_pane);

    snap.focused_workspace_id = Some("ws_2".into());
    snap.focused_tab_id = Some("tab_2".into());
    snap.focused_pane_id = Some("pane_2".into());
    snap
}

#[test]
fn navigator_start_search_focused_preset_focuses_the_query_field() {
    for start_search_focused in [false, true] {
        let mut config = Config::default();
        config.ui.navigator_start_search_focused = start_search_focused;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        state.set_snapshot(Box::new(snapshot()));
        state.set_pane_surface(surface());
        state.open_navigator_overlay();
        let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
            panic!("navigator should be open");
        };
        assert_eq!(
            navigator.search_focused, start_search_focused,
            "navigator_start_search_focused={start_search_focused}"
        );
    }
}

#[test]
fn agent_picker_start_search_focused_preset_focuses_the_query_field() {
    for start_search_focused in [false, true] {
        let mut config = Config::default();
        config.ui.agent_picker_start_search_focused = start_search_focused;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        state.set_snapshot(Box::new(snapshot()));
        state.set_pane_surface(surface());
        state.open_agent_picker_overlay();
        let Some(ClientShellOverlay::AgentPicker(picker)) = state.overlay.as_ref() else {
            panic!("agent picker should be open");
        };
        assert_eq!(
            picker.search_focused, start_search_focused,
            "agent_picker_start_search_focused={start_search_focused}"
        );
    }
}

#[test]
fn navigator_default_selection_targets_focused_workspace_or_pane() {
    let mut config = Config::default();
    config.ui.navigator_start_expanded = false;
    let snap = two_workspace_focused_on_second_snapshot();

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snap.clone()));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();

    // Collapsed: the workspace row carries the marker and owns the selection.
    let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
    else {
        panic!("expected navigator");
    };
    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_2".into(),
        })
    );
    assert_eq!(
        aggregate_navigation::navigator_selected_index(&rows, navigator),
        Some(1)
    );
    assert_eq!(rows.iter().filter(|row| row.current).count(), 1);
    assert!(rows[1].current);

    // Expanded: the focused pane takes the marker, so the popup opens on the pane.
    config.ui.navigator_start_expanded = true;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();

    let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
    else {
        panic!("expected navigator");
    };
    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Pane {
            endpoint_id: state.active_endpoint_id.clone(),
            pane_id: "pane_2".into(),
        })
    );
    let selected_index =
        aggregate_navigation::navigator_selected_index(&rows, navigator).expect("selected row");
    assert_eq!(
        rows[selected_index].target,
        navigator.selected.clone().unwrap()
    );
    assert_eq!(
        rows.iter().filter(|row| row.current).count(),
        1,
        "only the focused pane keeps the marker"
    );
    assert!(rows[selected_index].current);
    let ws_2_row = rows
        .iter()
        .find(|row| {
            matches!(
                &row.target,
                ClientNavigatorTarget::Workspace { workspace_id, .. } if workspace_id == "ws_2"
            )
        })
        .expect("ws_2 row");
    assert!(
        !ws_2_row.current,
        "the workspace row must yield the marker to its focused pane"
    );
}

#[test]
fn navigator_aligns_status_icons_for_current_and_other_panes() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    snap.workspaces[0].label = "main".into();
    let mut a0 = agent("alpha", AgentStatus::Working, 1);
    a0.pane_id = "pane_1".into();
    a0.focused = true;
    let mut pane2 = snap.panes[0].clone();
    pane2.pane_id = "pane_2".into();
    pane2.focused = false;
    let mut a1 = agent("beta", AgentStatus::Idle, 2);
    a1.pane_id = "pane_2".into();
    a1.focused = false;
    snap.panes = vec![snap.panes[0].clone(), pane2];
    snap.agents = vec![a0, a1];
    snap.focused_pane_id = Some("pane_1".into());
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());
    state.open_navigator_overlay();

    let frame = state.compose(106, 30).expect("navigator frame");
    let mut status_x = Vec::new();
    for (rect, target) in &state.hits.navigator_rows {
        if !matches!(target, ClientNavigatorTarget::Pane { .. }) {
            continue;
        }
        let y = rect.y as usize;
        let row = &frame.cells[y * frame.width as usize..(y + 1) * frame.width as usize];
        let x = row
            .iter()
            .position(|cell| cell.symbol == "●" || cell.symbol == "○" || cell.symbol == "·")
            .expect("status icon in pane row");
        status_x.push(x);
    }
    assert_eq!(status_x.len(), 2);
    assert_eq!(
        status_x[0], status_x[1],
        "status icons should start at the same offset whether or not the row is current"
    );
}

#[test]
fn navigator_uses_configured_status_indicator_style() {
    use crate::config::StatusIndicatorStyle;
    for (style, blocked, working, forbidden) in [
        (StatusIndicatorStyle::Dots, "●", "●", "×"),
        (StatusIndicatorStyle::Symbols, "×", "◐", "●"),
    ] {
        let mut config = Config::default();
        config.ui.status_indicators = style;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        let mut snap = snapshot();
        let mut a0 = agent("blocked-agent", AgentStatus::Blocked, 4);
        a0.pane_id = "pane_1".into();
        let mut pane2 = snap.panes[0].clone();
        pane2.pane_id = "pane_2".into();
        pane2.focused = false;
        let mut a1 = agent("working-agent", AgentStatus::Working, 3);
        a1.pane_id = "pane_2".into();
        a1.focused = false;
        snap.panes = vec![snap.panes[0].clone(), pane2];
        snap.agents = vec![a0, a1];
        state.set_snapshot(Box::new(snap));
        state.set_pane_surface(surface());
        state.open_navigator_overlay();

        let frame = state.compose(106, 30).expect("navigator frame");
        let text = frame
            .cells
            .chunks(frame.width as usize)
            .map(|row| row.iter().map(|c| c.symbol.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(blocked),
            "{style:?} should render {blocked:?}"
        );
        assert!(
            text.contains(working),
            "{style:?} should render {working:?}"
        );
        assert!(
            !text.contains(forbidden),
            "{style:?} must not render {forbidden:?}"
        );
    }
}

#[test]
fn navigator_ctrl_n_and_p_move_the_selection_in_browse_mode() {
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_1".into(),
        });
    }
    press_navigator_mod(&mut state, KeyCode::Char('n'), KeyModifiers::CONTROL);
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Pane {
            endpoint_id: state.active_endpoint_id.clone(),
            pane_id: "pane_1".into(),
        }),
        "ctrl+n steps forward to the next row"
    );
    press_navigator_mod(&mut state, KeyCode::Char('p'), KeyModifiers::CONTROL);
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Workspace {
            endpoint_id: state.active_endpoint_id.clone(),
            workspace_id: "ws_1".into(),
        }),
        "ctrl+p steps back"
    );
}

#[test]
fn navigator_f_clears_filters_and_keeps_the_current_selection() {
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    // Apply a status filter, which hides the idle pane rows.
    press_navigator_key(&mut state, KeyCode::Char('b'));
    assert!(
        navigator_pane_keys(&state).is_empty(),
        "idle rows are filtered out"
    );
    press_navigator_key(&mut state, KeyCode::Char('F'));
    assert_eq!(
        navigator_pane_keys(&state),
        vec!["pane_1", "pane_2"],
        "F clears the status filter"
    );
    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator should stay open");
    };
    assert!(
        navigator.query.as_str().is_empty(),
        "F clears the search query too"
    );
    assert_eq!(
        navigator.selected,
        Some(ClientNavigatorTarget::Pane {
            endpoint_id: state.active_endpoint_id.clone(),
            pane_id: "pane_1".into(),
        }),
        "F puts the selection back on the current row, not the top"
    );
}

#[test]
fn navigator_plain_f_does_not_clear_filters() {
    let mut state = navigator_multi_workspace_state(true);
    state.open_navigator_overlay();
    press_navigator_key(&mut state, KeyCode::Char('b'));
    assert!(navigator_pane_keys(&state).is_empty());
    press_navigator_key(&mut state, KeyCode::Char('f'));
    assert!(
        navigator_pane_keys(&state).is_empty(),
        "plain f must leave the filter in place"
    );
}

/// Open the navigator, select `target`, and press `x`.
fn navigator_press_x_on(
    state: &mut ClientShellState,
    target: ClientNavigatorTarget,
) -> ClientShellInput {
    state.open_navigator_overlay();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(target);
    }
    press_navigator_key(state, KeyCode::Char('x'))
}

fn local_pane_target(pane_id: &str) -> ClientNavigatorTarget {
    ClientNavigatorTarget::Pane {
        endpoint_id: ClientEndpointId::Local,
        pane_id: pane_id.to_owned(),
    }
}

#[test]
fn navigator_x_confirm_cancel_returns_to_navigator() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    assert!(state.config.confirm_pane_close);
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let outcome = navigator_press_x_on(&mut state, local_pane_target("pane_1"));
    assert!(outcome.actions.is_empty());
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
    let outcome = press_navigator_key(&mut state, KeyCode::Esc);
    assert!(outcome.actions.is_empty());
    let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
    else {
        panic!("esc should return to the navigator");
    };
    assert_eq!(
        navigator.selected,
        Some(local_pane_target("pane_1")),
        "cancelling leaves the selection where it was"
    );
}

#[test]
fn navigator_x_workspace_confirm_cancel_returns_to_navigator() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_close = true;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let outcome = navigator_press_x_on(
        &mut state,
        ClientNavigatorTarget::Workspace {
            endpoint_id: ClientEndpointId::Local,
            workspace_id: "ws_1".into(),
        },
    );
    assert!(outcome.actions.is_empty());
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::ConfirmClose(
            ClientConfirmCloseOverlay {
                tab_target: None,
                pane_target: None,
                return_to_navigator: Some(_),
                ..
            }
        ))
    ));
    let outcome = press_navigator_key(&mut state, KeyCode::Esc);
    assert!(outcome.actions.is_empty());
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::Navigator(_))),
        "esc should return to the navigator instead of the navigation sidebar"
    );
    assert_ne!(state.mode, ClientShellMode::Navigate);
}

#[test]
fn navigator_x_confirm_accept_keeps_navigator_open() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    assert!(state.config.confirm_pane_close);
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let outcome = navigator_press_x_on(&mut state, local_pane_target("pane_1"));
    assert!(outcome.actions.is_empty());
    let outcome = press_navigator_key(&mut state, KeyCode::Enter);
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!(
            "enter should confirm the pane close, got {:?}",
            outcome.actions
        );
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_1"
    ));
    let ClientShellOverlay::Navigator(navigator) = state.overlay.as_ref().expect("navigator")
    else {
        panic!("confirming should keep the navigator open");
    };
    // Upstream removed tab rows, so the row after the closed pane is the
    // workspace row, not a sibling tab.
    let rows =
        render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
    let selected = aggregate_navigation::navigator_selected_index(&rows, navigator)
        .expect("a row stays selected");
    assert!(
        rows[selected].target != local_pane_target("pane_1"),
        "the closed pane must not stay selected, got {:?}",
        rows[selected].target
    );
    assert_eq!(
        rows[selected].target,
        ClientNavigatorTarget::Workspace {
            endpoint_id: ClientEndpointId::Local,
            workspace_id: "ws_1".into(),
        },
        "the next row after the closed pane is the workspace row"
    );
}

#[test]
fn navigator_x_closes_directly_when_pane_confirmation_is_off() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_pane_close = false;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let outcome = navigator_press_x_on(&mut state, local_pane_target("pane_1"));
    let [ClientShellAction::Endpoint {
        endpoint_id,
        request,
        ..
    }] = &outcome.actions[..]
    else {
        panic!(
            "pane close should use endpoint API, got {:?}",
            outcome.actions
        );
    };
    assert_eq!(endpoint_id, &ClientEndpointId::Local);
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_1"
    ));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
}

#[test]
fn navigator_x_on_a_machine_row_does_nothing() {
    let (mut state, _remote) = state_with_remote();
    state.open_navigator_overlay();
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Machine {
            endpoint_id: ClientEndpointId::Local,
        });
    }
    let outcome = press_navigator_key(&mut state, KeyCode::Char('x'));
    assert!(
        outcome.actions.is_empty(),
        "machine rows are headers and cannot be closed"
    );
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
}

#[test]
fn navigator_x_on_a_foreign_workspace_targets_that_machine() {
    let (mut state, remote) = state_with_remote();
    let mut remote_snapshot = snapshot();
    remote_snapshot.boot_id = "remote-boot".into();
    remote_snapshot.workspaces[0].workspace_id = "ws_remote".into();
    remote_snapshot.workspaces[0].label = "remote-workspace".into();
    state.set_endpoint_snapshot(&remote, Box::new(remote_snapshot));
    state.open_navigator_overlay();

    let outcome = navigator_press_x_on(
        &mut state,
        ClientNavigatorTarget::Workspace {
            endpoint_id: remote.clone(),
            workspace_id: "ws_remote".into(),
        },
    );
    assert!(outcome.actions.is_empty(), "the dialog should open instead");
    let Some(ClientShellOverlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("expected confirm dialog, got {:?}", state.overlay);
    };
    assert_eq!(
        confirm.endpoint_id, remote,
        "the dialog must carry the remote machine, not the active one"
    );
    assert_eq!(confirm.workspace_id, "ws_remote");

    // Accepting dispatches WorkspaceClose to that machine, not the active one.
    let outcome = press_navigator_key(&mut state, KeyCode::Enter);
    let [ClientShellAction::Endpoint {
        endpoint_id,
        request,
        ..
    }] = &outcome.actions[..]
    else {
        panic!("expected an endpoint request, got {:?}", outcome.actions);
    };
    assert_eq!(
        endpoint_id, &remote,
        "WorkspaceClose must be addressed to the row's machine"
    );
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceClose(params)
            if params.workspace_id == "ws_remote" && !params.close_group
    ));
}

#[test]
fn navigator_x_on_a_foreign_pane_without_confirmation_targets_that_machine() {
    let (mut state, remote) = state_with_remote();
    state.config.confirm_pane_close = false;
    let mut remote_snapshot = snapshot();
    remote_snapshot.boot_id = "remote-boot".into();
    remote_snapshot.panes[0].pane_id = "pane_remote".into();
    remote_snapshot.panes[0].workspace_id = "ws_1".into();
    state.set_endpoint_snapshot(&remote, Box::new(remote_snapshot));
    state.open_navigator_overlay();

    let outcome = navigator_press_x_on(
        &mut state,
        ClientNavigatorTarget::Pane {
            endpoint_id: remote.clone(),
            pane_id: "pane_remote".into(),
        },
    );
    let [ClientShellAction::Endpoint {
        endpoint_id,
        request,
        ..
    }] = &outcome.actions[..]
    else {
        panic!("expected an endpoint request, got {:?}", outcome.actions);
    };
    assert_eq!(endpoint_id, &remote);
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneClose(params) if params.pane_id == "pane_remote"
    ));
}

#[test]
fn agent_picker_keybind_opens_overlay() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());

    // Default binding is prefix+shift+a, distinct from prefix+a for the
    // sidebar Agents section.
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    ))]);
    assert_eq!(state.mode, ClientShellMode::Prefix);

    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('A'),
        KeyModifiers::SHIFT,
    ))]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::AgentPicker(_))
    ));
}

#[test]
fn agent_picker_uses_compact_popup_geometry() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_agent_picker_overlay();

    // The picker is a compact popup: 76 columns, height proportional to the
    // terminal and clamped to 14..=22. Asserted so the geometry cannot silently
    // regress to the full-area margin sizing.
    state.compose(160, 48).expect("agent picker");
    let popup = state.hits.agent_picker_popup;
    assert_eq!(popup.width, 76);
    assert_eq!(popup.height, 22);
    assert!(
        popup.x > 0 && popup.y > 0,
        "compact popup should be inset, got {popup:?}"
    );
}

#[test]
fn agent_picker_selection_moves_with_jk_and_ctrl_np() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    snap.agents = vec![
        agent("agent 0", AgentStatus::Blocked, 3),
        agent("agent 1", AgentStatus::Working, 2),
        agent("agent 2", AgentStatus::Idle, 1),
    ];
    snap.agents[0].pane_id = "pane_1".into();
    snap.agents[1].pane_id = "pane_2".into();
    snap.agents[2].pane_id = "pane_3".into();
    snap.panes = snap
        .agents
        .iter()
        .map(|a| ClientShellPane {
            pane_id: a.pane_id.clone(),
            focused: a.pane_id == "pane_1",
            ..snap.panes[0].clone()
        })
        .collect();
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());

    state.open_agent_picker_overlay();

    let selected = |state: &ClientShellState| {
        let ClientShellOverlay::AgentPicker(picker) = state.overlay.as_ref().expect("agent picker")
        else {
            panic!("expected agent picker");
        };
        let rows = render::client_agent_picker_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            state.config.agent_panel_sort,
            picker,
        );
        aggregate_navigation::agent_picker_selected_index(&rows, picker)
    };

    assert_eq!(selected(&state), Some(0));

    let press = |state: &mut ClientShellState, code, modifiers| {
        state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
            code, modifiers,
        ))]);
    };

    // 'j' moves down
    press(&mut state, KeyCode::Char('j'), KeyModifiers::empty());
    assert_eq!(selected(&state), Some(1));

    // Ctrl+n moves down
    press(&mut state, KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert_eq!(selected(&state), Some(2));

    // 'k' moves up
    press(&mut state, KeyCode::Char('k'), KeyModifiers::empty());
    assert_eq!(selected(&state), Some(1));

    // Ctrl+p moves up
    press(&mut state, KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert_eq!(selected(&state), Some(0));

    // Down and Up arrows
    press(&mut state, KeyCode::Down, KeyModifiers::empty());
    assert_eq!(selected(&state), Some(1));
    press(&mut state, KeyCode::Up, KeyModifiers::empty());
    assert_eq!(selected(&state), Some(0));
}

#[test]
fn agent_picker_query_and_filter_narrows_agents() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    snap.workspaces[0].label = "frontend".into();
    let mut a0 = agent("compiler", AgentStatus::Blocked, 4);
    a0.pane_id = "pane_1".into();
    a0.title = Some("build error in parser".into());

    let mut a1 = agent("linter", AgentStatus::Working, 3);
    a1.pane_id = "pane_2".into();
    a1.title = Some("checking types".into());

    let mut a2 = agent("server", AgentStatus::Idle, 2);
    a2.pane_id = "pane_3".into();
    a2.title = Some("listening on 8080".into());

    snap.agents = vec![a0, a1, a2];
    snap.panes = snap
        .agents
        .iter()
        .map(|a| ClientShellPane {
            pane_id: a.pane_id.clone(),
            focused: a.pane_id == "pane_1",
            ..snap.panes[0].clone()
        })
        .collect();
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());

    state.open_agent_picker_overlay();

    let row_names = |state: &ClientShellState| {
        let ClientShellOverlay::AgentPicker(picker) = state.overlay.as_ref().expect("agent picker")
        else {
            panic!("expected agent picker");
        };
        render::client_agent_picker_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            state.config.agent_panel_sort,
            picker,
        )
        .into_iter()
        .map(|r| r.agent_label)
        .collect::<Vec<_>>()
    };

    assert_eq!(row_names(&state), vec!["compiler", "linter", "server"]);

    let press = |state: &mut ClientShellState, code, modifiers| {
        state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
            code, modifiers,
        ))]);
    };

    // Filter by status when !search_focused: 'b' for Blocked
    press(&mut state, KeyCode::Char('b'), KeyModifiers::empty());
    assert_eq!(row_names(&state), vec!["compiler"]);

    // Filter 'w' for Working
    press(&mut state, KeyCode::Char('w'), KeyModifiers::empty());
    assert_eq!(row_names(&state), vec!["linter"]);

    // Filter 'i' for Idle
    press(&mut state, KeyCode::Char('i'), KeyModifiers::empty());
    assert_eq!(row_names(&state), vec!["server"]);

    // 'F' clears filter
    press(&mut state, KeyCode::Char('F'), KeyModifiers::empty());
    assert_eq!(row_names(&state), vec!["compiler", "linter", "server"]);

    // Focus search with '/' and search by title
    press(&mut state, KeyCode::Char('/'), KeyModifiers::empty());
    state.handle_input_bytes(b"parser");
    assert_eq!(row_names(&state), vec!["compiler"]);

    // Search by workspace label
    if let Some(ClientShellOverlay::AgentPicker(picker)) = state.overlay.as_mut() {
        picker.query = TextEditor::from("frontend");
    }
    assert_eq!(row_names(&state), vec!["compiler", "linter", "server"]);

    // Search by status string
    if let Some(ClientShellOverlay::AgentPicker(picker)) = state.overlay.as_mut() {
        picker.query = TextEditor::from("working");
    }
    assert_eq!(row_names(&state), vec!["linter"]);
}

#[test]
fn agent_picker_search_keystrokes_repaint() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    let mut a0 = agent("compiler", AgentStatus::Idle, 1);
    a0.pane_id = "pane_1".into();
    let mut a1 = agent("linter", AgentStatus::Working, 2);
    a1.pane_id = "pane_2".into();
    snap.agents = vec![a0, a1];
    snap.panes = snap
        .agents
        .iter()
        .map(|a| ClientShellPane {
            pane_id: a.pane_id.clone(),
            focused: a.pane_id == "pane_1",
            ..snap.panes[0].clone()
        })
        .collect();
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());
    state.open_agent_picker_overlay();

    let press = |state: &mut ClientShellState, code, modifiers| {
        state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
            code, modifiers,
        ))])
    };

    // Enter search mode.
    let outcome = press(&mut state, KeyCode::Char('/'), KeyModifiers::empty());
    assert!(outcome.repaint, "opening search must repaint");
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::AgentPicker(picker)) if picker.search_focused
    ));

    // Typing filters the rows and must schedule a repaint, otherwise the
    // popup looks frozen while the query state already changed.
    let outcome = press(&mut state, KeyCode::Char('c'), KeyModifiers::empty());
    assert!(
        outcome.repaint,
        "typing in agent picker search must repaint"
    );
    let query = match state.overlay.as_ref() {
        Some(ClientShellOverlay::AgentPicker(picker)) => picker.query.as_str().to_owned(),
        _ => panic!("expected agent picker"),
    };
    assert_eq!(query, "c");

    // Editing keys share the same path: backspace must repaint too.
    let outcome = press(&mut state, KeyCode::Backspace, KeyModifiers::empty());
    assert!(
        outcome.repaint,
        "backspace in agent picker search must repaint"
    );

    // Cursor-only moves change no content but move the visible cursor.
    press(&mut state, KeyCode::Char('x'), KeyModifiers::empty());
    let outcome = press(&mut state, KeyCode::Left, KeyModifiers::empty());
    assert!(
        outcome.repaint,
        "cursor move in agent picker search must repaint"
    );
}

#[test]
fn agent_picker_fuzzy_subsequence_ranks_prefix_first() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    let mut nested = agent("my-compiler", AgentStatus::Idle, 2);
    nested.pane_id = "pane_1".into();
    let mut exact = agent("compiler", AgentStatus::Idle, 1);
    exact.pane_id = "pane_2".into();
    snap.agents = vec![nested, exact];
    snap.panes = snap
        .agents
        .iter()
        .map(|a| ClientShellPane {
            pane_id: a.pane_id.clone(),
            focused: a.pane_id == "pane_1",
            ..snap.panes[0].clone()
        })
        .collect();
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());

    state.open_agent_picker_overlay();

    let row_names = |state: &ClientShellState| {
        let ClientShellOverlay::AgentPicker(picker) = state.overlay.as_ref().expect("agent picker")
        else {
            panic!("expected agent picker");
        };
        render::client_agent_picker_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            state.config.agent_panel_sort,
            picker,
        )
        .into_iter()
        .map(|r| r.agent_label)
        .collect::<Vec<_>>()
    };

    // Subsequence (not substring) still finds both agents.
    if let Some(ClientShellOverlay::AgentPicker(picker)) = state.overlay.as_mut() {
        picker.query = TextEditor::from("cmplr");
    }
    assert_eq!(row_names(&state), vec!["compiler", "my-compiler"]);

    // Prefix match outranks the mid-word match.
    if let Some(ClientShellOverlay::AgentPicker(picker)) = state.overlay.as_mut() {
        picker.query = TextEditor::from("comp");
    }
    assert_eq!(row_names(&state), vec!["compiler", "my-compiler"]);
}

#[test]
fn agent_picker_enter_activates_selected_agent_pane() {
    let (mut state, endpoint_id) = state_with_scrollable_agents();
    state.open_agent_picker_overlay();

    // Select remote agent
    if let Some(ClientShellOverlay::AgentPicker(picker)) = state.overlay.as_mut() {
        picker.selected = Some((endpoint_id.clone(), "pane_1".into()));
    }

    let mut outcome = ClientShellInput::default();
    state.accept_agent_picker_selection(&mut outcome);

    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint {
            endpoint_id: activated,
            target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
        }] if activated == &endpoint_id && pane_id == "pane_1"
    ));
    assert!(state.overlay.is_none());

    // Also test via Enter key event
    state.open_agent_picker_overlay();
    assert!(state.overlay.is_some());
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Enter,
        KeyModifiers::empty(),
    ))]);
    assert!(state.overlay.is_none());
}

#[test]
fn agent_picker_renders_header_rows_and_footer() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    snap.workspaces[0].label = "main".into();
    let mut a = agent("pi", AgentStatus::Working, 1);
    a.pane_id = "pane_1".into();
    a.tokens = vec![(
        crate::api::schema::AGENT_STATUS_CHANGED_UNIX_MS_TOKEN.into(),
        (super::super::agent_sidebar::current_unix_ms().saturating_sub(120_000)).to_string(),
    )];
    snap.agents = vec![a];
    snap.panes = vec![ClientShellPane {
        pane_id: "pane_1".into(),
        focused: true,
        ..snap.panes[0].clone()
    }];
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());
    state.open_agent_picker_overlay();

    let frame = state.compose(106, 30).expect("agent picker frame");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| row.iter().map(|c| c.symbol.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(text.contains("search agents"));
    assert!(text.contains("1 agent"));
    assert!(text.contains("pi"));
    assert!(text.contains("main · 1"));
    assert!(text.contains("2m"));
    assert!(text.contains("filter F/b/w/i/d"));
}

#[test]
fn agent_picker_shift_f_clears_filter_and_reselects_current_agent() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    let mut a0 = agent("compiler", AgentStatus::Blocked, 4);
    a0.pane_id = "pane_1".into();
    let mut a1 = agent("linter", AgentStatus::Working, 3);
    a1.pane_id = "pane_2".into();
    let mut a2 = agent("server", AgentStatus::Idle, 2);
    a2.pane_id = "pane_3".into();

    snap.agents = vec![a0, a1, a2];
    snap.focused_pane_id = Some("pane_2".into());
    snap.panes = snap
        .agents
        .iter()
        .map(|a| ClientShellPane {
            pane_id: a.pane_id.clone(),
            focused: a.pane_id == "pane_2",
            ..snap.panes[0].clone()
        })
        .collect();
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());

    state.open_agent_picker_overlay();

    let selected_pane = |state: &ClientShellState| {
        let ClientShellOverlay::AgentPicker(picker) = state.overlay.as_ref().expect("agent picker")
        else {
            panic!("expected agent picker");
        };
        picker.selected.as_ref().map(|(_, pane_id)| pane_id.clone())
    };

    assert_eq!(selected_pane(&state), Some("pane_2".into()));

    let press = |state: &mut ClientShellState, code, modifiers| {
        state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
            code, modifiers,
        ))]);
    };

    // Filter 'i' for Idle (only pane_3 matches)
    press(&mut state, KeyCode::Char('i'), KeyModifiers::empty());
    state.move_agent_picker_selection(0);
    assert_eq!(selected_pane(&state), Some("pane_3".into()));

    // Lowercase 'f' without Shift does NOT clear filter
    press(&mut state, KeyCode::Char('f'), KeyModifiers::empty());
    assert_eq!(selected_pane(&state), Some("pane_3".into()));

    // Uppercase 'F' clears filter and re-selects current agent (pane_2)
    press(&mut state, KeyCode::Char('F'), KeyModifiers::empty());
    assert_eq!(selected_pane(&state), Some("pane_2".into()));

    // Also verify with Char('f') and Shift modifier
    press(&mut state, KeyCode::Char('b'), KeyModifiers::empty());
    state.move_agent_picker_selection(0);
    assert_eq!(selected_pane(&state), Some("pane_1".into()));

    press(&mut state, KeyCode::Char('f'), KeyModifiers::SHIFT);
    assert_eq!(selected_pane(&state), Some("pane_2".into()));
}

#[test]
fn agent_picker_respects_agent_panel_sort_config() {
    let mut config = Config::default();
    config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut snap = snapshot();
    let mut a0 = agent("idle-agent", AgentStatus::Idle, 1);
    a0.pane_id = "pane_1".into();
    let mut a1 = agent("blocked-agent", AgentStatus::Blocked, 2);
    a1.pane_id = "pane_2".into();

    snap.agents = vec![a0, a1];
    snap.panes = snap
        .agents
        .iter()
        .map(|a| ClientShellPane {
            pane_id: a.pane_id.clone(),
            focused: a.pane_id == "pane_1",
            ..snap.panes[0].clone()
        })
        .collect();
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());

    state.open_agent_picker_overlay();

    let row_names = |state: &ClientShellState| {
        let ClientShellOverlay::AgentPicker(picker) = state.overlay.as_ref().expect("agent picker")
        else {
            panic!("expected agent picker");
        };
        render::client_agent_picker_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            state.config.agent_panel_sort,
            picker,
        )
        .into_iter()
        .map(|r| r.agent_label)
        .collect::<Vec<_>>()
    };

    // Under Spaces sort, preserves order: idle-agent first
    assert_eq!(row_names(&state), vec!["idle-agent", "blocked-agent"]);

    // Change sort to Priority
    state.config.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    // Under Priority sort, Blocked comes before Idle
    assert_eq!(row_names(&state), vec!["blocked-agent", "idle-agent"]);
}
#[test]
fn agent_picker_aligns_agent_column_for_uneven_workspace_names() {
    let mut config = Config::default();
    config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut snap = snapshot();
    let mut ws_a = snap.workspaces[0].clone();
    ws_a.workspace_id = "ws_a".into();
    ws_a.label = "a".into();
    ws_a.active_tab_id = "tab_a".into();
    ws_a.focused = true;
    let mut ws_b = snap.workspaces[0].clone();
    ws_b.workspace_id = "ws_b".into();
    ws_b.label = "very-long-workspace-name-indeed".into();
    ws_b.active_tab_id = "tab_b".into();
    ws_b.focused = false;
    snap.workspaces = vec![ws_a, ws_b];
    let mut tab_a = snap.tabs[0].clone();
    tab_a.tab_id = "tab_a".into();
    tab_a.workspace_id = "ws_a".into();
    let mut tab_b = snap.tabs[0].clone();
    tab_b.tab_id = "tab_b".into();
    tab_b.workspace_id = "ws_b".into();
    tab_b.focused = false;
    snap.tabs = vec![tab_a, tab_b];
    let mut pane_a = snap.panes[0].clone();
    pane_a.pane_id = "pane_1".into();
    pane_a.workspace_id = "ws_a".into();
    pane_a.tab_id = "tab_a".into();
    pane_a.focused = true;
    let mut pane_b = snap.panes[0].clone();
    pane_b.pane_id = "pane_2".into();
    pane_b.workspace_id = "ws_b".into();
    pane_b.tab_id = "tab_b".into();
    pane_b.focused = false;
    snap.panes = vec![pane_a, pane_b];
    snap.focused_workspace_id = Some("ws_a".into());
    snap.focused_tab_id = Some("tab_a".into());
    snap.focused_pane_id = Some("pane_1".into());
    let mut a0 = agent("alpha", AgentStatus::Idle, 1);
    a0.pane_id = "pane_1".into();
    a0.workspace_id = "ws_a".into();
    a0.tab_id = "tab_a".into();
    a0.focused = true;
    let mut a1 = agent("averylongagentname", AgentStatus::Idle, 2);
    a1.pane_id = "pane_2".into();
    a1.workspace_id = "ws_b".into();
    a1.tab_id = "tab_b".into();
    a1.focused = false;
    snap.agents = vec![a0, a1];
    state.set_snapshot(Box::new(snap));
    state.set_pane_surface(surface());
    state.open_agent_picker_overlay();

    let frame = state.compose(106, 30).expect("agent picker frame");
    let lines: Vec<String> = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| row.iter().map(|c| c.symbol.as_str()).collect::<String>())
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("very-long-workspace-name-indeed · 1")),
        "workspace column should retain labels up to its 36-cell limit"
    );
    let mut starts = Vec::new();
    for (rect, _, pane_id) in &state.hits.agent_picker_rows {
        // Agent column caps at 12: "averylongagentname" renders truncated.
        let label = if pane_id == "pane_1" {
            "alpha"
        } else {
            "averylongag…"
        };
        let start = rect.y as usize * frame.width as usize + rect.x as usize;
        let cells = &frame.cells[start..start + rect.width as usize];
        let symbols = label.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();
        let pos = cells
            .windows(symbols.len())
            .position(|window| {
                window
                    .iter()
                    .zip(&symbols)
                    .all(|(cell, symbol)| cell.symbol == *symbol)
            })
            .expect("agent label in row");
        starts.push(pos);
    }
    assert_eq!(starts.len(), 2);
    assert_eq!(
        starts[0], starts[1],
        "agent column should start at the same offset in every row"
    );
}
