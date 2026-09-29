//! Correctness of the per-endpoint grouped-ordering memo.

use super::*;
use crate::client::endpoint::ProfileId;
use crate::protocol::ClientShellWorktree;

/// Workspaces in three worktree groups: a non-linked parent plus two linked
/// members each, one standalone workspace, and a second group of two.
fn grouped_snapshot() -> ClientShellSnapshot {
    let base = snapshot();
    let mut projected = base.clone();
    projected.workspaces = (1..=8)
        .map(|number| {
            let mut workspace = base.workspaces[0].clone();
            workspace.workspace_id = format!("ws_{number}");
            workspace.number = number;
            workspace.label = format!("work-{number}");
            workspace.branch = Some(format!("branch-{number}"));
            workspace.focused = number == 1;
            workspace.worktree = match number {
                1..=3 => Some(worktree("repo-a", number != 1)),
                4..=5 => Some(worktree("repo-b", number != 4)),
                _ => None,
            };
            workspace
        })
        .collect();
    projected
}

fn worktree(key: &str, is_linked_worktree: bool) -> ClientShellWorktree {
    ClientShellWorktree {
        key: key.into(),
        label: key.into(),
        is_linked_worktree,
    }
}

fn selections() -> Vec<HashSet<String>> {
    ["repo-a", "repo-b", "repo-a\nrepo-b", ""]
        .iter()
        .map(|keys| {
            keys.split('\n')
                .filter(|key| !key.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

fn local_state(projected: ClientShellSnapshot) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state
}

/// Every selection must answer exactly like the uncached ordering, and the
/// fully expanded accessor must ignore the endpoint's own collapse set.
fn assert_memo_matches_uncached(state: &ClientShellState, snapshot: &ClientShellSnapshot) {
    let endpoint = state.active_endpoint().expect("local endpoint");
    for collapsed in selections() {
        assert_eq!(
            *endpoint.workspace_entries(snapshot, &collapsed),
            super::super::workspace_entries::workspace_entries(snapshot, &collapsed),
            "collapsed selection {collapsed:?}"
        );
    }
    assert_eq!(
        *endpoint.expanded_workspace_entries(snapshot),
        super::super::workspace_entries::workspace_entries(snapshot, &HashSet::new()),
    );
}

#[test]
fn memo_matches_uncached_ordering_for_every_collapse_selection() {
    let projected = grouped_snapshot();
    let mut state = local_state(projected.clone());
    state.refresh_workspace_entries();
    assert_memo_matches_uncached(&state, &projected);
    // Without a refresh the memo is empty, so every read must still be right.
    let unrefreshed = local_state(projected.clone());
    assert_memo_matches_uncached(&unrefreshed, &projected);
}

#[test]
fn memo_follows_focus_moves_across_a_collapsed_group() {
    let mut projected = grouped_snapshot();
    let mut state = local_state(projected.clone());
    state.collapsed_groups = ["repo-a".to_owned()].into_iter().collect();
    state.refresh_workspace_entries();
    assert_memo_matches_uncached(&state, &projected);

    for member in 2..=3 {
        projected.workspaces[member - 1].focused = true;
        projected.workspaces[0].focused = false;
        state.set_snapshot(Box::new(projected.clone()));
        state.refresh_workspace_entries();
        assert_memo_matches_uncached(&state, &projected);
    }
}

#[test]
fn memo_follows_workspace_add_remove_and_worktree_changes() {
    let mut projected = grouped_snapshot();
    let mut state = local_state(projected.clone());
    state.collapsed_groups = ["repo-a".to_owned(), "repo-b".to_owned()]
        .into_iter()
        .collect();
    state.refresh_workspace_entries();
    assert_memo_matches_uncached(&state, &projected);

    // A workspace that joins a group turns a standalone entry into a child.
    projected.workspaces[5].worktree = Some(worktree("repo-a", true));
    state.set_snapshot(Box::new(projected.clone()));
    state.refresh_workspace_entries();
    assert_memo_matches_uncached(&state, &projected);

    // Removing the group parent leaves only linked members, which stop grouping.
    projected.workspaces[0].worktree = None;
    state.set_snapshot(Box::new(projected.clone()));
    state.refresh_workspace_entries();
    assert_memo_matches_uncached(&state, &projected);

    // A group that loses a member is no longer a group at all.
    let removed = projected.workspaces.remove(2);
    assert_eq!(removed.workspace_id, "ws_3");
    state.set_snapshot(Box::new(projected.clone()));
    state.refresh_workspace_entries();
    assert_memo_matches_uncached(&state, &projected);
}

#[test]
fn memo_follows_a_remote_endpoints_own_collapse_set() {
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").expect("profile id"),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
    let mut projected = grouped_snapshot();
    projected.boot_id = "remote-boot".into();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
    state.set_snapshot(Box::new(grouped_snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_snapshot(&endpoint_id, Box::new(projected.clone()));
    state.refresh_workspace_entries();

    for collapsed_groups in [
        HashSet::new(),
        ["repo-a".to_owned()].into_iter().collect(),
        ["repo-b".to_owned()].into_iter().collect(),
    ] {
        let remote = state
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == endpoint_id)
            .expect("remote endpoint");
        assert_eq!(
            *remote.workspace_entries(&projected, &collapsed_groups),
            super::super::workspace_entries::workspace_entries(&projected, &collapsed_groups),
            "collapsed selection {collapsed_groups:?}"
        );
    }

    // The local collapse set must not leak into the remote endpoint's ordering.
    state.collapsed_groups = ["repo-a".to_owned(), "repo-b".to_owned()]
        .into_iter()
        .collect();
    state.refresh_workspace_entries();
    let remote = state
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .expect("remote endpoint");
    assert_eq!(
        *remote.workspace_entries(&projected, &HashSet::new()),
        super::super::workspace_entries::workspace_entries(&projected, &HashSet::new()),
    );
    assert_eq!(
        *remote.expanded_workspace_entries(&projected),
        super::super::workspace_entries::workspace_entries(&projected, &HashSet::new()),
    );
}

#[test]
fn memo_never_answers_for_a_snapshot_it_was_not_built_from() {
    let projected = grouped_snapshot();
    let state = local_state(projected.clone());
    let endpoint = state.active_endpoint().expect("local endpoint");
    let mut other = projected.clone();
    other.workspaces[1].branch = Some("renamed".into());
    assert_eq!(
        *endpoint.workspace_entries(&other, &HashSet::new()),
        super::super::workspace_entries::workspace_entries(&other, &HashSet::new()),
    );
    let mut shorter = projected.clone();
    shorter.workspaces.truncate(4);
    assert_eq!(
        *endpoint.workspace_entries(&shorter, &HashSet::new()),
        super::super::workspace_entries::workspace_entries(&shorter, &HashSet::new()),
    );
}
