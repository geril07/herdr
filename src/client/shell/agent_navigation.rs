use super::*;

/// A client-only preview. Snapshot identity prevents Enter from using a reused pane ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentNavigationTarget {
    pub(super) endpoint_id: ClientEndpointId,
    pub(super) pane_id: String,
    boot_id: String,
    generation: Option<u64>,
}

impl AgentNavigationTarget {
    pub(super) fn matches(&self, endpoint_id: &ClientEndpointId, pane_id: &str) -> bool {
        &self.endpoint_id == endpoint_id && self.pane_id == pane_id
    }

    #[cfg(test)]
    pub(super) fn for_test(
        endpoint_id: ClientEndpointId,
        pane_id: String,
        boot_id: String,
        generation: Option<u64>,
    ) -> Self {
        Self {
            endpoint_id,
            pane_id,
            boot_id,
            generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SidebarNavSection {
    #[default]
    Spaces,
    Agents,
}

impl SidebarNavSection {
    pub(super) fn toggle(self) -> Self {
        match self {
            Self::Spaces => Self::Agents,
            Self::Agents => Self::Spaces,
        }
    }
}

impl ClientShellState {
    /// Agent walk order matches the sidebar: aggregate order across online
    /// endpoints, which is the single-endpoint sidebar order when only one
    /// endpoint is present.
    pub(super) fn navigation_agent_targets(&self) -> Vec<AgentNavigationTarget> {
        super::aggregate_navigation::online_agent_targets(
            &self.endpoints,
            &self.active_endpoint_id,
            self.config.agent_panel_sort,
        )
        .into_iter()
        .filter_map(|target| {
            let endpoint = self
                .endpoints
                .iter()
                .find(|entry| entry.endpoint_id == target.endpoint_id)?;
            let snapshot = endpoint.snapshot.as_deref()?;
            (endpoint.status == ClientEndpointStatus::Online
                && snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == target.pane_id))
            .then(|| AgentNavigationTarget {
                endpoint_id: target.endpoint_id,
                pane_id: target.pane_id,
                boot_id: snapshot.boot_id.clone(),
                generation: endpoint.snapshot_generation,
            })
        })
        .collect()
    }

    pub(super) fn focused_agent_target(&self) -> Option<AgentNavigationTarget> {
        let pane_id = self.snapshot.as_deref()?.focused_pane_id.clone()?;
        self.navigation_agent_targets().into_iter().find(|target| {
            target.endpoint_id == self.active_endpoint_id && target.pane_id == pane_id
        })
    }

    pub(super) fn initial_agent_target(&self) -> Option<AgentNavigationTarget> {
        self.focused_agent_target()
            .or_else(|| self.navigation_agent_targets().into_iter().next())
    }

    pub(super) fn navigation_agent_valid(&self, target: &AgentNavigationTarget) -> bool {
        self.endpoints.iter().any(|endpoint| {
            endpoint.endpoint_id == target.endpoint_id
                && endpoint.status == ClientEndpointStatus::Online
                && endpoint.snapshot_generation == target.generation
                && endpoint.snapshot.as_deref().is_some_and(|snapshot| {
                    snapshot.boot_id == target.boot_id
                        && snapshot
                            .agents
                            .iter()
                            .any(|agent| agent.pane_id == target.pane_id)
                })
        })
    }

    pub(super) fn navigation_preview_action_blocked(&self) -> bool {
        self.workspace_preview_blocked() || self.agent_preview_blocked()
    }

    pub(super) fn workspace_preview_blocked(&self) -> bool {
        self.navigate_workspace_id.as_ref().is_some_and(|target| {
            target.endpoint_id != self.active_endpoint_id || !self.navigation_target_valid(target)
        })
    }

    pub(super) fn agent_preview_blocked(&self) -> bool {
        self.navigate_agent.as_ref().is_some_and(|target| {
            target.endpoint_id != self.active_endpoint_id || !self.navigation_agent_valid(target)
        })
    }

    pub(super) fn active_section_blocked(&self) -> bool {
        match self.navigate_section {
            SidebarNavSection::Spaces => self.workspace_preview_blocked(),
            SidebarNavSection::Agents => self.agent_preview_blocked(),
        }
    }

    pub(super) fn push_active_section_blocked_notice(&mut self) {
        let _ = match self.navigate_section {
            SidebarNavSection::Spaces => self.push_endpoint_notice(
                ClientEndpointNoticeKind::Rejected,
                "navigate_endpoint_inactive",
                "Confirm workspace first",
                "Select an available workspace and press Enter before using workspace or pane actions",
            ),
            SidebarNavSection::Agents => self.push_endpoint_notice(
                ClientEndpointNoticeKind::Rejected,
                "navigate_agent_endpoint_inactive",
                "Confirm agent first",
                "Select an available agent and press Enter before using agent actions",
            ),
        };
    }

    pub(super) fn enter_navigate_mode(&mut self, section: SidebarNavSection) {
        self.mobile_switcher_scroll = 0;
        self.reveal_mobile_workspace = false;
        self.mode = ClientShellMode::Navigate;
        self.navigate_workspace_id = self.focused_navigation_target();
        self.navigate_agent = self.initial_agent_target();
        self.navigate_section = section;
        self.reveal_navigation_workspace = true;
        if section == SidebarNavSection::Agents {
            if let Some(index) = self.navigate_agent.as_ref().and_then(|selected| {
                self.navigation_agent_targets()
                    .iter()
                    .position(|target| target == selected)
            }) {
                // Render clamps to the live max; stale hit state must not pin this.
                self.agent_scroll = index;
            }
        }
    }

    pub(super) fn clear_navigate_preview(&mut self) {
        self.navigate_workspace_id = None;
        self.navigate_agent = None;
        self.navigate_section = SidebarNavSection::Spaces;
    }

    pub(super) fn switch_navigate_section(&mut self) {
        self.navigate_section = self.navigate_section.toggle();
        match self.navigate_section {
            SidebarNavSection::Spaces => {
                if self.navigate_workspace_id.is_none() {
                    self.navigate_workspace_id = self.focused_navigation_target();
                }
                if let Some(target) = self.navigate_workspace_id.clone() {
                    self.reveal_workspace(&target.workspace_id);
                }
                self.reveal_navigation_workspace = true;
            }
            SidebarNavSection::Agents => {
                if self.navigate_agent.is_none() {
                    self.navigate_agent = self.initial_agent_target();
                }
                if let Some(index) = self.navigate_agent.as_ref().and_then(|selected| {
                    self.navigation_agent_targets()
                        .iter()
                        .position(|target| target == selected)
                }) {
                    // Render clamps to the live max; stale hit state must not pin this.
                    self.agent_scroll = index;
                }
            }
        }
    }

    pub(super) fn move_navigate_agent(&mut self, delta: isize) {
        let mobile = self.mobile_layout_active();
        let targets = self.navigation_agent_targets();
        if targets.is_empty() {
            return;
        }
        let current = self
            .navigate_agent
            .as_ref()
            .and_then(|selected| targets.iter().position(|target| target == selected));
        let next = match current {
            Some(current) if mobile => {
                (current as isize + delta).clamp(0, targets.len() as isize - 1) as usize
            }
            Some(current) => (current as isize + delta).rem_euclid(targets.len() as isize) as usize,
            None if delta < 0 => targets.len() - 1,
            None => 0,
        };
        let target = targets[next].clone();
        self.collapsed_endpoints.remove(&target.endpoint_id);
        // Render clamps to the live max; stale hit state must not pin this.
        self.agent_scroll = next;
        self.navigate_agent = Some(target);
    }

    pub(super) fn accept_navigate_agent(&mut self, outcome: &mut ClientShellInput) {
        let Some(target) = self.navigate_agent.clone() else {
            self.mode = self.copy_or_terminal_mode();
            outcome.repaint = true;
            return;
        };
        if !self.navigation_agent_valid(&target) {
            self.receive_endpoint_unavailable(
                "Agent is no longer available; select a connected agent".into(),
            );
            outcome.repaint = true;
            return;
        }
        if self.focus_or_activate(
            target.endpoint_id,
            ClientEndpointFocusTarget::Pane(target.pane_id),
            outcome,
        ) {
            self.mode = ClientShellMode::Terminal;
            self.clear_navigate_preview();
        }
        outcome.repaint = true;
    }
}
