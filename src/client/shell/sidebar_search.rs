use super::*;

#[derive(Default)]
pub(super) struct SidebarSearch {
    pub(super) spaces: Option<SectionSearch>,
    pub(super) agents: Option<SectionSearch>,
    presentation: Option<SearchPresentation>,
}

struct SearchPresentation {
    spaces: SpacesSidebarConfig,
    agents: crate::config::AgentsSidebarConfig,
}

#[derive(Default)]
pub(super) struct SectionSearch {
    pub(super) query: TextEditor,
    parsed: Option<super::fuzzy::FuzzyQuery>,
    // Revisioned snapshots let idle frames reuse matches without rescoring every pane.
    // The minute bucket expires the elapsed-time token that rows redraw over time.
    sources: Vec<(ClientEndpointId, String, String, u64, u64)>,
    matches: HashMap<ClientEndpointId, HashSet<String>>,
}

impl SidebarSearch {
    pub(super) fn section(&self, section: SidebarNavSection) -> Option<&SectionSearch> {
        match section {
            SidebarNavSection::Spaces => self.spaces.as_ref(),
            SidebarNavSection::Agents => self.agents.as_ref(),
        }
    }

    pub(super) fn section_mut(&mut self, section: SidebarNavSection) -> &mut Option<SectionSearch> {
        match section {
            SidebarNavSection::Spaces => &mut self.spaces,
            SidebarNavSection::Agents => &mut self.agents,
        }
    }

    pub(super) fn refresh(
        &mut self,
        endpoints: &[ClientShellEndpoint],
        config: &ClientShellConfig,
    ) -> bool {
        if self.spaces.is_none() && self.agents.is_none() {
            return false;
        }
        let presentation_changed = self
            .presentation
            .as_ref()
            .is_none_or(|old| old.spaces != config.spaces || old.agents != config.agents);
        if presentation_changed {
            if let Some(search) = self.spaces.as_mut() {
                search.sources.clear();
            }
            if let Some(search) = self.agents.as_mut() {
                search.sources.clear();
            }
            self.presentation = Some(SearchPresentation {
                spaces: config.spaces.clone(),
                agents: config.agents.clone(),
            });
        }
        let mut changed = false;
        if let Some(search) = self.spaces.as_mut() {
            changed |= search.refresh(endpoints, config, SidebarNavSection::Spaces);
        }
        if let Some(search) = self.agents.as_mut() {
            changed |= search.refresh(endpoints, config, SidebarNavSection::Agents);
        }
        changed
    }
}

impl SectionSearch {
    pub(super) fn invalidate(&mut self) {
        self.parsed =
            (!self.query.trim().is_empty()).then(|| super::fuzzy::parse_query(&self.query));
        self.sources.clear();
    }

    pub(super) fn filtering(&self) -> bool {
        self.parsed.is_some()
    }

    pub(super) fn includes(&self, endpoint: &ClientEndpointId, id: &str) -> bool {
        !self.filtering()
            || self
                .matches
                .get(endpoint)
                .is_some_and(|ids| ids.contains(id))
    }

    fn refresh(
        &mut self,
        endpoints: &[ClientShellEndpoint],
        config: &ClientShellConfig,
        section: SidebarNavSection,
    ) -> bool {
        let Some(query) = self.parsed.as_ref() else {
            return false;
        };
        let sources = || {
            endpoints.iter().filter_map(|endpoint| {
                let snapshot = endpoint.snapshot.as_deref()?;
                Some((endpoint, snapshot))
            })
        };
        let now = super::agent_sidebar::current_unix_ms();
        let minute = match section {
            SidebarNavSection::Spaces => 0,
            SidebarNavSection::Agents => now / 60_000,
        };
        if self.sources.len() == sources().count()
            && self
                .sources
                .iter()
                .zip(sources())
                .all(|(old, (endpoint, snapshot))| {
                    old.0 == endpoint.endpoint_id
                        && old.1 == endpoint.label
                        && old.2 == snapshot.boot_id
                        && old.3 == snapshot.revision
                        && old.4 == minute
                })
        {
            return false;
        }
        self.sources.clear();
        self.matches.clear();
        let mut scratch = super::fuzzy::FuzzyScratch::new();
        for (endpoint, snapshot) in sources() {
            self.sources.push((
                endpoint.endpoint_id.clone(),
                endpoint.label.clone(),
                snapshot.boot_id.clone(),
                snapshot.revision,
                minute,
            ));
            let mut ids = HashSet::new();
            match section {
                SidebarNavSection::Spaces => {
                    for entry in endpoint.expanded_workspace_entries(snapshot).iter() {
                        let workspace = &snapshot.workspaces[entry.index];
                        let rows = super::sidebar::workspace_rows(
                            workspace,
                            workspace.agent_status,
                            entry.indented,
                            &config.spaces,
                        );
                        if matches_rows(&rows, query, &mut scratch) {
                            ids.insert(workspace.workspace_id.clone());
                        }
                    }
                }
                SidebarNavSection::Agents => {
                    for agent in &snapshot.agents {
                        let Some(row) = super::agent_sidebar::agent_row(
                            snapshot,
                            &agent.pane_id,
                            config,
                            Some(&endpoint.label),
                            now,
                        ) else {
                            continue;
                        };
                        if matches_rows(&row.rows, query, &mut scratch) {
                            ids.insert(agent.pane_id.clone());
                        }
                    }
                }
            }
            self.matches.insert(endpoint.endpoint_id.clone(), ids);
        }
        true
    }

    pub(super) fn highlight(&self, buffer: &mut Buffer, rect: Rect, palette: &Palette) {
        let Some(query) = self.parsed.as_ref() else {
            return;
        };
        for y in rect.y..rect.bottom() {
            let mut label = String::new();
            let mut positions = Vec::new();
            for x in rect.x..rect.right() {
                let symbol = buffer[(x, y)].symbol();
                label.push_str(symbol);
                positions.extend(symbol.chars().map(|_| x));
            }
            for (x, matched) in positions
                .into_iter()
                .zip(super::fuzzy::highlight_for_label(&label, query))
            {
                if matched {
                    buffer[(x, y)]
                        .set_fg(palette.accent)
                        .set_style(Style::default().add_modifier(Modifier::BOLD));
                }
            }
        }
    }
}

fn matches_rows(
    rows: &[Vec<crate::ui::ResolvedToken>],
    query: &super::fuzzy::FuzzyQuery,
    scratch: &mut super::fuzzy::FuzzyScratch,
) -> bool {
    let fields = rows
        .iter()
        .flatten()
        .filter_map(|token| token.kind.text_value())
        .collect::<Vec<_>>();
    super::fuzzy::match_any_field(&fields, query, scratch).is_some()
}

pub(super) fn includes(
    search: Option<&SectionSearch>,
    endpoint: &ClientEndpointId,
    id: &str,
) -> bool {
    search.is_none_or(|search| search.includes(endpoint, id))
}

pub(super) fn render_controls(
    buffer: &mut Buffer,
    header: Rect,
    search: Option<&SectionSearch>,
    focused: bool,
    count: usize,
    palette: &Palette,
) -> Rect {
    let Some(search) = search else {
        return Rect::default();
    };
    if header.width < 8 || header.height < 2 {
        return Rect::default();
    }
    // The query line indents like the section title and its first row.
    let row = Rect::new(header.x, header.y + 1, header.width, 1);
    let suffix = format!(" {count} [×]");
    let suffix_width = super::render::display_width(&suffix).min(row.width.saturating_sub(4));
    super::render::put_text(
        buffer,
        row.x + 1,
        row.y,
        2,
        "/ ",
        Style::default().fg(palette.accent),
    );
    let input = Rect::new(
        row.x + 3,
        row.y,
        row.width.saturating_sub(suffix_width + 3),
        1,
    );
    let cursor = super::text_editor::render(
        buffer,
        input,
        &search.query,
        Style::default().fg(palette.text),
    );
    if focused {
        if let Some(cursor) = cursor {
            buffer[(cursor.x, cursor.y)]
                .set_style(Style::default().fg(palette.sidebar_bg).bg(palette.accent));
        }
    }
    super::render::put_right_text(
        buffer,
        row,
        row.y,
        &suffix,
        Style::default().fg(palette.overlay0),
    );
    Rect::new(row.right().saturating_sub(3), row.y, 3, 1)
}

impl ClientShellState {
    pub(super) fn finish_sidebar_search(&mut self) {
        if self.sidebar_search.spaces.is_some() || self.sidebar_search.agents.is_some() {
            self.clear_navigate_preview();
            self.mode = ClientShellMode::Terminal;
        }
    }

    pub(super) fn sidebar_search_focused(&self) -> bool {
        self.mode == ClientShellMode::Navigate
            && self.overlay.is_none()
            && self.sidebar_search.section(self.navigate_section).is_some()
    }

    pub(super) fn open_sidebar_search(
        &mut self,
        section: SidebarNavSection,
        outcome: &mut ClientShellInput,
    ) {
        if self.mode != ClientShellMode::Navigate {
            self.enter_navigate_mode(section);
        }
        self.navigate_section = section;
        self.chrome_drag = None;
        self.workspace_press = None;
        self.tab_press = None;
        self.sidebar_search
            .section_mut(section)
            .get_or_insert_with(SectionSearch::default);
        if self.sidebar_collapsed {
            self.sidebar_collapsed = false;
            self.invalidate_pane_surface();
            outcome.resize = true;
        }
        outcome.repaint = true;
    }

    pub(super) fn clear_sidebar_search(&mut self, section: SidebarNavSection) {
        *self.sidebar_search.section_mut(section) = None;
        match section {
            SidebarNavSection::Spaces => {
                self.workspace_scroll = 0;
                self.navigate_workspace_id = self.focused_navigation_target();
                self.reveal_navigation_workspace = true;
            }
            SidebarNavSection::Agents => {
                self.navigate_agent = self.initial_agent_target();
                self.agent_scroll = 0;
                self.reveal_selected_navigate_agent();
            }
        }
    }

    pub(super) fn refresh_sidebar_search(&mut self) {
        if self.mode != ClientShellMode::Navigate || self.mobile_layout_active() {
            self.sidebar_search = SidebarSearch::default();
            return;
        }
        self.sidebar_search.refresh(&self.endpoints, &self.config);
        self.reconcile_sidebar_search_selection();
    }

    fn reconcile_sidebar_search_selection(&mut self) {
        let invalid = match self.navigate_section {
            SidebarNavSection::Spaces => {
                self.sidebar_search.spaces.is_some()
                    && self.navigate_workspace_id.as_ref().is_none_or(|target| {
                        !self.navigation_target_valid(target)
                            || !includes(
                                self.sidebar_search.spaces.as_ref(),
                                &target.endpoint_id,
                                &target.workspace_id,
                            )
                    })
            }
            SidebarNavSection::Agents => {
                self.sidebar_search.agents.is_some()
                    && self.navigate_agent.as_ref().is_none_or(|target| {
                        !self.navigation_agent_valid(target)
                            || !includes(
                                self.sidebar_search.agents.as_ref(),
                                &target.endpoint_id,
                                &target.pane_id,
                            )
                    })
            }
        };
        if invalid {
            match self.navigate_section {
                SidebarNavSection::Spaces => {
                    self.navigate_workspace_id = None;
                    self.move_navigate_workspace(1);
                }
                SidebarNavSection::Agents => {
                    self.navigate_agent = None;
                    self.move_navigate_agent(1);
                }
            }
        }
    }

    pub(super) fn sidebar_query_changed(&mut self) {
        if let Some(search) = self
            .sidebar_search
            .section_mut(self.navigate_section)
            .as_mut()
        {
            search.invalidate();
        }
        self.sidebar_search.refresh(&self.endpoints, &self.config);
        match self.navigate_section {
            SidebarNavSection::Spaces => {
                self.workspace_scroll = 0;
                self.navigate_workspace_id = None;
                self.move_navigate_workspace(1);
            }
            SidebarNavSection::Agents => {
                self.agent_scroll = 0;
                self.navigate_agent = None;
                self.move_navigate_agent(1);
            }
        }
    }

    pub(super) fn route_sidebar_search_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if !self.sidebar_search_focused() || self.config.keybinds.matches_prefix(key) {
            return false;
        }
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        if code == KeyCode::Esc || super::input::is_ctrl_bracket_key(key) {
            self.clear_sidebar_search(self.navigate_section);
        } else if code == KeyCode::Enter && modifiers.is_empty() {
            match self.navigate_section {
                SidebarNavSection::Spaces if self.navigate_workspace_id.is_some() => {
                    self.accept_navigate_workspace(outcome)
                }
                SidebarNavSection::Agents if self.navigate_agent.is_some() => {
                    self.accept_navigate_agent(outcome)
                }
                _ => {}
            }
        } else if (code == KeyCode::Tab && modifiers.is_empty())
            || (code == KeyCode::BackTab
                && modifiers
                    .difference(crossterm::event::KeyModifiers::SHIFT)
                    .is_empty())
        {
            self.switch_navigate_section();
            self.reconcile_sidebar_search_selection();
        } else if (modifiers.is_empty() && matches!(code, KeyCode::Up | KeyCode::Down))
            || (modifiers == crossterm::event::KeyModifiers::CONTROL
                && matches!(code, KeyCode::Char('n' | 'p')))
        {
            let delta = if matches!(code, KeyCode::Up | KeyCode::Char('p')) {
                -1
            } else {
                1
            };
            match self.navigate_section {
                SidebarNavSection::Spaces => self.move_navigate_workspace(delta),
                SidebarNavSection::Agents => self.move_navigate_agent(delta),
            }
        } else if let Some(search) = self
            .sidebar_search
            .section_mut(self.navigate_section)
            .as_mut()
        {
            if search.query.handle_key(key) == Some(true) {
                self.sidebar_query_changed();
            }
        }
        outcome.repaint = true;
        true
    }
}
