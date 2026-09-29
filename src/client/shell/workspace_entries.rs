//! Grouped workspace ordering and its per-endpoint memo.
//!
//! The ordering is a pure function of two inputs: the worktree shape of a
//! snapshot's workspace list, and the set of collapsed worktree group keys.
//! Every surface that lists spaces needs it, several times per frame, so each
//! endpoint memoizes the orderings it is asked for and rebuilds them when those
//! inputs change.
//!
//! Validity is decided by comparing the live inputs with the ones the memo was
//! built from, never by a revision counter or by remembering which code path
//! replaced the snapshot. A memo that was never refreshed, or that was refreshed
//! from a different snapshot, therefore costs a recompute instead of answering
//! wrongly.

use super::*;
use std::borrow::Cow;

/// The facts `workspace_entries` reads from one workspace, in snapshot order.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkspaceGroupingFact {
    worktree_key: Option<Box<str>>,
    is_linked_worktree: bool,
    focused: bool,
}

#[derive(Clone, Debug, Default)]
struct WorkspaceGroupingShape {
    workspaces: Vec<WorkspaceGroupingFact>,
}

impl WorkspaceGroupingShape {
    fn capture(snapshot: &ClientShellSnapshot) -> Self {
        Self {
            workspaces: snapshot
                .workspaces
                .iter()
                .map(|workspace| {
                    let worktree = workspace.worktree.as_ref();
                    WorkspaceGroupingFact {
                        worktree_key: worktree.map(|worktree| worktree.key.as_str().into()),
                        is_linked_worktree: worktree
                            .is_some_and(|worktree| worktree.is_linked_worktree),
                        focused: workspace.focused,
                    }
                })
                .collect(),
        }
    }

    fn matches(&self, snapshot: &ClientShellSnapshot) -> bool {
        self.workspaces.len() == snapshot.workspaces.len()
            && self
                .workspaces
                .iter()
                .zip(&snapshot.workspaces)
                .all(|(cached, workspace)| {
                    let worktree = workspace.worktree.as_ref();
                    cached.is_linked_worktree
                        == worktree.is_some_and(|worktree| worktree.is_linked_worktree)
                        && cached.focused == workspace.focused
                        && cached.worktree_key.as_deref()
                            == worktree.map(|worktree| worktree.key.as_str())
                })
    }
}

/// Grouped orderings for one endpoint, one per collapsed-group selection the
/// shell asks for. Slot 0 is fully expanded; slot 1 is the endpoint's own
/// collapsed selection and stays empty when that selection has no keys, because
/// slot 0 already answers it.
#[derive(Clone, Debug, Default)]
pub(super) struct EndpointWorkspaceEntries {
    shape: WorkspaceGroupingShape,
    selections: [Option<Vec<String>>; 2],
    orderings: [Vec<WorkspaceEntry>; 2],
}

impl EndpointWorkspaceEntries {
    /// The memoized ordering for these inputs, or the freshly computed one when
    /// the memo describes something else.
    pub(super) fn get<'e>(
        &'e self,
        snapshot: &ClientShellSnapshot,
        collapsed_groups: &HashSet<String>,
    ) -> Cow<'e, [WorkspaceEntry]> {
        if self.shape.matches(snapshot) {
            if let Some(slot) = self.slot(collapsed_groups) {
                return Cow::Borrowed(&self.orderings[slot]);
            }
        }
        Cow::Owned(workspace_entries(snapshot, collapsed_groups))
    }

    /// Rebuild the orderings whose inputs changed since the last refresh.
    pub(super) fn refresh(
        &mut self,
        snapshot: &ClientShellSnapshot,
        collapsed_groups: &HashSet<String>,
    ) {
        if !self.shape.matches(snapshot) {
            self.shape = WorkspaceGroupingShape::capture(snapshot);
            self.selections = [None, None];
        }
        self.rebuild(0, snapshot, &HashSet::new());
        if collapsed_groups.is_empty() {
            self.selections[1] = None;
            self.orderings[1].clear();
        } else {
            self.rebuild(1, snapshot, collapsed_groups);
        }
    }

    fn slot(&self, collapsed_groups: &HashSet<String>) -> Option<usize> {
        self.selections.iter().position(|selection| {
            selection.as_ref().is_some_and(|keys| {
                keys.len() == collapsed_groups.len()
                    && keys.iter().all(|key| collapsed_groups.contains(key))
            })
        })
    }

    fn rebuild(
        &mut self,
        slot: usize,
        snapshot: &ClientShellSnapshot,
        collapsed_groups: &HashSet<String>,
    ) {
        if self.slot(collapsed_groups) == Some(slot) {
            return;
        }
        self.selections[slot] = Some(collapsed_groups.iter().cloned().collect());
        self.orderings[slot] = workspace_entries(snapshot, collapsed_groups);
    }
}

impl ClientShellEndpoint {
    /// The grouped ordering for `snapshot` on this endpoint, memoized against
    /// the exact workspace shape and collapsed-group keys it was built from.
    pub(super) fn workspace_entries<'e>(
        &'e self,
        snapshot: &ClientShellSnapshot,
        collapsed_groups: &HashSet<String>,
    ) -> Cow<'e, [WorkspaceEntry]> {
        self.workspace_entries.get(snapshot, collapsed_groups)
    }

    /// The fully expanded grouping, where no group hides its members. The agents
    /// panel and every machine-level list use this so their order stays stable
    /// while the sidebar collapses groups.
    pub(super) fn expanded_workspace_entries(
        &self,
        snapshot: &ClientShellSnapshot,
    ) -> Cow<'_, [WorkspaceEntry]> {
        self.workspace_entries(snapshot, &HashSet::new())
    }
}

/// The orderings the single-endpoint spaces surfaces draw: the sidebar's
/// collapsed view and the fully expanded view the agents panel aligns to. Both
/// borrow from the active endpoint's memo so a caller can still mutate render
/// state while the sidebar walks them.
pub(super) fn active_workspace_orderings<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    collapsed_groups: &'a HashSet<String>,
    snapshot: &ClientShellSnapshot,
) -> super::render::EndpointWorkspaceOrderings<'a> {
    let active = endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == active_endpoint_id);
    super::render::EndpointWorkspaceOrderings {
        collapsed: active.map_or_else(
            || Cow::Owned(workspace_entries(snapshot, collapsed_groups)),
            |endpoint| endpoint.workspace_entries(snapshot, collapsed_groups),
        ),
        expanded: active.map_or_else(
            || Cow::Owned(workspace_entries(snapshot, &HashSet::new())),
            |endpoint| endpoint.expanded_workspace_entries(snapshot),
        ),
    }
}

pub(crate) fn workspace_entries(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
) -> Vec<WorkspaceEntry> {
    let mut members = HashMap::<&str, Vec<usize>>::new();
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        if let Some(worktree) = &workspace.worktree {
            members.entry(&worktree.key).or_default().push(index);
        }
    }
    let grouped = members
        .iter()
        .filter(|(_, indices)| {
            indices.len() >= 2
                && indices.iter().any(|index| {
                    snapshot.workspaces[*index]
                        .worktree
                        .as_ref()
                        .is_some_and(|worktree| !worktree.is_linked_worktree)
                })
        })
        .map(|(key, _)| *key)
        .collect::<HashSet<_>>();
    let mut emitted = HashSet::<&str>::new();
    let mut entries = Vec::new();
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        let Some(worktree) = workspace
            .worktree
            .as_ref()
            .filter(|worktree| grouped.contains(worktree.key.as_str()))
        else {
            entries.push(WorkspaceEntry {
                index,
                indented: false,
                last_child: false,
            });
            continue;
        };
        if !emitted.insert(&worktree.key) {
            continue;
        }
        let Some(group_members) = members.get(worktree.key.as_str()) else {
            continue;
        };
        let parent = group_members
            .iter()
            .copied()
            .find(|member| {
                snapshot.workspaces[*member]
                    .worktree
                    .as_ref()
                    .is_some_and(|worktree| !worktree.is_linked_worktree)
            })
            .unwrap_or(index);
        entries.push(WorkspaceEntry {
            index: parent,
            indented: false,
            last_child: false,
        });
        if collapsed_groups.contains(&worktree.key) {
            if let Some(active) = group_members
                .iter()
                .copied()
                .find(|member| *member != parent && snapshot.workspaces[*member].focused)
            {
                entries.push(WorkspaceEntry {
                    index: active,
                    indented: true,
                    last_child: true,
                });
            }
            continue;
        }
        let children = group_members
            .iter()
            .copied()
            .filter(|member| *member != parent)
            .collect::<Vec<_>>();
        for (child_index, child) in children.iter().enumerate() {
            entries.push(WorkspaceEntry {
                index: *child,
                indented: true,
                last_child: child_index + 1 == children.len(),
            });
        }
    }
    entries
}
