use super::*;
use crate::api::schema::AgentStatus;
use crate::config::TabStatusOrderConfig;

const TAB_SCROLL_BUTTON_WIDTH: u16 = 3;
const MIN_TAB_STRIP_WIDTH: u16 =
    MIN_TAB_WIDTH + NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2);
fn tab_status_priority(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 4,
        AgentStatus::Working => 3,
        AgentStatus::Done => 2,
        AgentStatus::Idle => 1,
        AgentStatus::Unknown => 0,
    }
}

pub(crate) fn tab_agent_statuses(
    tab_id: &str,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
) -> Vec<AgentStatus> {
    if !config.tab_status || config.tab_status_max == 0 {
        return Vec::new();
    }

    let pane_pos = |pane_id: &str| -> usize {
        snapshot
            .panes
            .iter()
            .position(|p| p.pane_id == pane_id)
            .unwrap_or(usize::MAX)
    };

    let mut agents = snapshot
        .agents
        .iter()
        .filter(|agent| agent.tab_id == tab_id)
        .filter(|agent| {
            if config.tab_status_idle {
                true
            } else {
                matches!(
                    agent.agent_status,
                    AgentStatus::Working | AgentStatus::Blocked | AgentStatus::Done
                )
            }
        })
        .collect::<Vec<_>>();

    match config.tab_status_order {
        TabStatusOrderConfig::Physical => {
            agents.sort_by_key(|agent| pane_pos(&agent.pane_id));
        }
        TabStatusOrderConfig::Priority => {
            agents.sort_by_key(|agent| {
                (
                    std::cmp::Reverse(tab_status_priority(agent.agent_status)),
                    pane_pos(&agent.pane_id),
                )
            });
        }
    }

    agents
        .into_iter()
        .take(config.tab_status_max)
        .map(|agent| agent.agent_status)
        .collect()
}

pub(crate) fn tab_indicator_prefix_width(count: usize, spacing: bool) -> u16 {
    if count == 0 {
        0
    } else {
        let inner_width = if spacing {
            count.saturating_mul(2).saturating_sub(1)
        } else {
            count
        };
        inner_width.saturating_add(1).min(u16::MAX as usize) as u16
    }
}

fn tab_number(tab: &ClientShellTab, index: usize, config: &ClientShellConfig) -> Option<String> {
    (config.tab_bar_numbers && tab.custom_label).then(|| (index + 1).to_string())
}

fn tab_number_prefix_width(number: Option<&str>) -> u16 {
    number
        .map(|number| display_width(number).saturating_add(1))
        .unwrap_or(0)
}

pub(crate) fn render_tab_bar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    tab_scroll: &mut usize,
    reveal_focused_tab: &mut bool,
    tab_drag_insert_index: Option<usize>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    buffer.set_style(area, Style::default().bg(palette.panel_bg));
    let tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref())
        .collect::<Vec<_>>();
    let tab_statuses = tabs
        .iter()
        .map(|tab| tab_agent_statuses(&tab.tab_id, snapshot, config))
        .collect::<Vec<_>>();
    let tab_labels = tabs.iter().map(|tab| tab_label(tab)).collect::<Vec<_>>();
    let tab_numbers = tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| tab_number(tab, index, config))
        .collect::<Vec<_>>();
    let desired_widths = tab_labels
        .iter()
        .zip(&tab_numbers)
        .zip(&tab_statuses)
        .map(|((label, number), statuses)| {
            let indicator_width =
                tab_indicator_prefix_width(statuses.len(), config.tab_status_spacing);
            display_width(label)
                .saturating_add(indicator_width)
                .saturating_add(tab_number_prefix_width(number.as_deref()))
                .saturating_add(4)
                .max(MIN_TAB_WIDTH)
        })
        .collect::<Vec<_>>();
    let content = tab_bar_content_area(snapshot, area);
    let mouse_chrome = config.mouse_capture;
    let new_tab_width = if mouse_chrome { NEW_TAB_WIDTH } else { 0 };
    let desired_total = desired_widths
        .iter()
        .copied()
        .fold(0_u16, u16::saturating_add)
        .saturating_add(tabs.len().saturating_sub(1).min(u16::MAX as usize) as u16)
        .saturating_add(new_tab_width);
    let overflow =
        desired_total > content.width && (!mouse_chrome || content.width >= MIN_TAB_STRIP_WIDTH);
    let available = if overflow && mouse_chrome {
        content
            .width
            .saturating_sub(NEW_TAB_WIDTH)
            .saturating_sub(TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2))
    } else {
        content.width.saturating_sub(new_tab_width)
    };
    let max_scroll = max_tab_scroll(&desired_widths, available);
    if !overflow {
        *tab_scroll = 0;
    } else if *reveal_focused_tab {
        if let Some(focused) = tabs.iter().position(|tab| tab.focused) {
            *tab_scroll = centered_tab_scroll(focused, &desired_widths, available).min(max_scroll);
        }
    } else {
        *tab_scroll = (*tab_scroll).min(max_scroll);
    }
    *reveal_focused_tab = false;

    let mut x = content.x;
    let tab_right = if overflow && mouse_chrome {
        hits.tab_scroll_left = Rect::new(
            content.x,
            content.y,
            TAB_SCROLL_BUTTON_WIDTH.min(content.width),
            1,
        );
        put_text(
            buffer,
            hits.tab_scroll_left.x,
            content.y,
            hits.tab_scroll_left.width,
            " < ",
            Style::default()
                .fg(if *tab_scroll > 0 {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        x = hits.tab_scroll_left.right();
        content
            .right()
            .saturating_sub(NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH)
    } else {
        content.right().saturating_sub(new_tab_width)
    };

    let mut first_visible = None;
    let mut last_visible = None;
    for (index, tab) in tabs.iter().enumerate().skip(*tab_scroll) {
        let name = &tab_labels[index];
        let number = tab_numbers[index].as_deref();
        let statuses = &tab_statuses[index];
        let desired = desired_widths[index];
        let remaining = tab_right.saturating_sub(x);
        let width = desired.min(remaining);
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let style = if tab.focused {
            let base = Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent);
            if tab.custom_label {
                base.add_modifier(Modifier::BOLD)
            } else {
                base
            }
        } else if tab.custom_label {
            Style::default().fg(palette.overlay1).bg(palette.surface0)
        } else {
            Style::default().fg(palette.overlay0).bg(palette.surface0)
        };
        let tab_bg = if tab.focused {
            palette.accent
        } else {
            palette.surface0
        };

        buffer.set_style(rect, style);

        let indicator_prefix_width =
            tab_indicator_prefix_width(statuses.len(), config.tab_status_spacing);
        let number_prefix_width = tab_number_prefix_width(number);
        let total_content_width = display_width(name)
            .saturating_add(indicator_prefix_width)
            .saturating_add(number_prefix_width);
        let padding = width.saturating_sub(total_content_width);
        let left = padding / 2;
        let mut cur_x = rect.x.saturating_add(left);

        if !statuses.is_empty() {
            for (i, status) in statuses.iter().enumerate() {
                if i > 0 && config.tab_status_spacing {
                    cur_x = cur_x.saturating_add(1);
                }
                if cur_x < rect.right() {
                    let icon = status_icon(*status, config.status_indicators);
                    let icon_style = Style::default()
                        .fg(status_color(*status, palette))
                        .bg(tab_bg);
                    put_text(buffer, cur_x, rect.y, 1, icon, icon_style);
                }
                cur_x = cur_x.saturating_add(1);
            }
            cur_x = cur_x.saturating_add(1);
        }

        if let Some(number) = number {
            if cur_x < rect.right() {
                let avail = rect.right().saturating_sub(cur_x);
                put_text(buffer, cur_x, rect.y, avail, number, style);
            }
            cur_x = cur_x.saturating_add(display_width(number));
            if cur_x < rect.right() {
                put_text(buffer, cur_x, rect.y, 1, " ", style);
            }
            cur_x = cur_x.saturating_add(1);
        }

        if cur_x < rect.right() {
            let avail = rect.right().saturating_sub(cur_x);
            put_text(buffer, cur_x, rect.y, avail, name, style);
        }

        hits.tabs.push((rect, tab.tab_id.clone()));
        first_visible.get_or_insert(index);
        last_visible = Some(index);
        x = x.saturating_add(width + 1);
        if width < desired {
            break;
        }
    }

    if overflow && mouse_chrome {
        hits.tab_scroll_right = Rect::new(tab_right, area.y, TAB_SCROLL_BUTTON_WIDTH, 1);
        let can_scroll_right = *tab_scroll < max_scroll;
        put_text(
            buffer,
            hits.tab_scroll_right.x,
            area.y,
            hits.tab_scroll_right.width,
            " > ",
            Style::default()
                .fg(if can_scroll_right {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        hits.new_tab = Rect::new(
            hits.tab_scroll_right.right(),
            area.y,
            content
                .right()
                .saturating_sub(hits.tab_scroll_right.right())
                .min(NEW_TAB_WIDTH),
            1,
        );
    } else if mouse_chrome {
        hits.new_tab = Rect::new(
            x.min(content.right()),
            area.y,
            content.right().saturating_sub(x).min(NEW_TAB_WIDTH),
            1,
        );
    }
    if mouse_chrome {
        put_text(
            buffer,
            hits.new_tab.x,
            area.y,
            hits.new_tab.width,
            " + ",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }

    if first_visible.is_some_and(|index| index > 0) {
        let ellipsis_x = if hits.tab_scroll_left.width > 0 {
            hits.tab_scroll_left.right()
        } else {
            content.x
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }
    if last_visible.is_some_and(|index| index + 1 < tabs.len()) {
        let ellipsis_x = if hits.tab_scroll_right.width > 0 {
            hits.tab_scroll_right.x.saturating_sub(1)
        } else {
            content.right().saturating_sub(1)
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x >= content.x && ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }

    if let Some(insert_index) = tab_drag_insert_index {
        if let Some(indicator_x) = tab_drop_indicator_x(hits, &tabs, insert_index) {
            put_text(
                buffer,
                indicator_x.min(content.right().saturating_sub(1)),
                area.y,
                1,
                "│",
                Style::default().fg(palette.accent),
            );
        }
    }
    render_tab_bar_status(buffer, area, snapshot, palette);
}

pub(crate) fn tab_bar_status_width(snapshot: &ClientShellSnapshot) -> u16 {
    let content = snapshot.tab_bar_right.iter().fold(0u16, |width, segment| {
        width.saturating_add(display_width(&segment.text))
    });
    let separators = snapshot.tab_bar_right.len().saturating_sub(1);
    content.saturating_add(
        display_width(&snapshot.tab_bar_right_separator)
            .saturating_mul(separators.min(u16::MAX as usize) as u16),
    )
}

fn tab_bar_status_area(snapshot: &ClientShellSnapshot, area: Rect) -> Option<Rect> {
    let width = tab_bar_status_width(snapshot);
    if width == 0 {
        return None;
    }
    let reserved = width.saturating_add(1);
    (area.width.saturating_sub(reserved) >= MIN_TAB_STRIP_WIDTH)
        .then(|| Rect::new(area.right().saturating_sub(width), area.y, width, 1))
}

fn tab_bar_content_area(snapshot: &ClientShellSnapshot, area: Rect) -> Rect {
    let reserved = tab_bar_status_area(snapshot, area)
        .map(|status| status.width.saturating_add(1))
        .unwrap_or(0);
    Rect {
        width: area.width.saturating_sub(reserved),
        ..area
    }
}

fn render_tab_bar_status(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) {
    let Some(status) = tab_bar_status_area(snapshot, area) else {
        return;
    };
    let separator_width = display_width(&snapshot.tab_bar_right_separator);
    let mut x = status.x;
    for (index, segment) in snapshot.tab_bar_right.iter().enumerate() {
        if index > 0 && separator_width > 0 {
            put_text(
                buffer,
                x,
                area.y,
                separator_width,
                &snapshot.tab_bar_right_separator,
                Style::default().fg(palette.overlay0).bg(palette.panel_bg),
            );
            x = x.saturating_add(separator_width);
        }
        let width = display_width(&segment.text);
        let style = if segment.accent {
            Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay1).bg(palette.panel_bg)
        };
        put_text(buffer, x, area.y, width, &segment.text, style);
        x = x.saturating_add(width);
    }
}

fn tab_drop_indicator_x(
    hits: &ShellHitMap,
    tabs: &[&ClientShellTab],
    insert_index: usize,
) -> Option<u16> {
    let visible = hits
        .tabs
        .iter()
        .filter_map(|(rect, tab_id)| {
            tabs.iter()
                .position(|tab| tab.tab_id == *tab_id)
                .map(|index| (index, *rect))
        })
        .collect::<Vec<_>>();
    let (first_index, first_rect) = *visible.first()?;
    let (last_index, last_rect) = *visible.last()?;
    if insert_index == 0 {
        return Some(if first_index == 0 {
            first_rect.x
        } else {
            hits.tab_scroll_left.right()
        });
    }
    if let Some((_, rect)) = visible.iter().find(|(index, _)| *index == insert_index) {
        return Some(rect.x.saturating_sub(1));
    }
    if insert_index >= tabs.len() {
        return Some(if last_index + 1 >= tabs.len() {
            last_rect.right()
        } else {
            hits.tab_scroll_right.x.saturating_sub(1)
        });
    }
    None
}

fn centered_tab_scroll(focused: usize, widths: &[u16], available: u16) -> usize {
    let mut best = focused;
    let mut best_distance = u16::MAX;
    for start in 0..=focused {
        let before = widths
            .iter()
            .copied()
            .enumerate()
            .skip(start)
            .take(focused.saturating_sub(start))
            .fold(0u16, |width, (_, tab)| width.saturating_add(tab + 1));
        if before >= available {
            continue;
        }
        let focused_width = widths[focused].min(available.saturating_sub(before));
        let center = before.saturating_mul(2).saturating_add(focused_width);
        let distance = center.abs_diff(available);
        if distance <= best_distance {
            best_distance = distance;
            best = start;
        }
    }
    best
}

fn max_tab_scroll(widths: &[u16], available: u16) -> usize {
    let Some((&last, preceding)) = widths.split_last() else {
        return 0;
    };
    let mut start = preceding.len();
    let mut used = u32::from(last);
    // Keep the longest fully visible suffix, not merely a sliver of the last tab.
    // An oversized last tab must still be reachable at the start of the strip.
    for width in preceding.iter().rev() {
        let required = used + 1 + u32::from(*width);
        if required > u32::from(available) {
            break;
        }
        used = required;
        start -= 1;
    }
    start
}

fn tab_label(tab: &ClientShellTab) -> String {
    if tab.zoomed {
        format!("{} Z", tab.label)
    } else {
        tab.label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::protocol::{
        ClientShellAgent, ClientShellPane, ClientShellTab, ClientShellWorkspace,
    };

    fn make_agent(
        pane_id: &str,
        workspace_id: &str,
        tab_id: &str,
        status: AgentStatus,
    ) -> ClientShellAgent {
        ClientShellAgent {
            pane_id: pane_id.into(),
            workspace_id: workspace_id.into(),
            tab_id: tab_id.into(),
            name: None,
            display_agent: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: status,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
        }
    }

    fn make_test_snapshot() -> ClientShellSnapshot {
        ClientShellSnapshot {
            boot_id: "boot".into(),
            revision: 1,
            config_diagnostic: None,
            product_announcement: None,
            update_available: None,
            update_install_command: "herdr update".into(),
            server_keybindings_toml: None,
            latest_release_notes_available: false,
            integration_updates_available: false,
            worktree_directory: "/tmp".into(),
            release_notes: None,
            focused_workspace_id: Some("ws_1".into()),
            focused_tab_id: Some("tab_1".into()),
            focused_pane_id: Some("pane_1".into()),
            tab_bar_right: Vec::new(),
            tab_bar_right_separator: " ".into(),
            agent_view_label: None,
            agent_order: Vec::new(),
            workspaces: vec![ClientShellWorkspace {
                workspace_id: "ws_1".into(),
                active_tab_id: "tab_1".into(),
                new_workspace_cwd: "/repo".into(),
                number: 1,
                label: "main".into(),
                custom_label: false,
                branch: None,
                git_ahead_behind: None,
                tokens: Vec::new(),
                worktree: None,
                focused: true,
                agent_status: AgentStatus::Idle,
            }],
            tabs: vec![
                ClientShellTab {
                    focused: true,
                    tab_id: "tab_1".into(),
                    workspace_id: "ws_1".into(),
                    number: 1,
                    label: "1".into(),
                    custom_label: false,
                    zoomed: false,
                    agent_status: AgentStatus::Idle,
                },
                ClientShellTab {
                    focused: false,
                    tab_id: "tab_2".into(),
                    workspace_id: "ws_1".into(),
                    number: 2,
                    label: "2".into(),
                    custom_label: false,
                    zoomed: false,
                    agent_status: AgentStatus::Idle,
                },
                ClientShellTab {
                    focused: false,
                    tab_id: "tab_3".into(),
                    workspace_id: "ws_1".into(),
                    number: 3,
                    label: "3".into(),
                    custom_label: false,
                    zoomed: false,
                    agent_status: AgentStatus::Idle,
                },
            ],
            panes: vec![
                ClientShellPane {
                    pane_id: "pane_1".into(),
                    workspace_id: "ws_1".into(),
                    tab_id: "tab_1".into(),
                    label: None,
                    cwd: None,
                    foreground_cwd: None,
                    focused: true,
                    right_click_passthrough: false,
                },
                ClientShellPane {
                    pane_id: "pane_2a".into(),
                    workspace_id: "ws_1".into(),
                    tab_id: "tab_2".into(),
                    label: None,
                    cwd: None,
                    foreground_cwd: None,
                    focused: false,
                    right_click_passthrough: false,
                },
                ClientShellPane {
                    pane_id: "pane_2b".into(),
                    workspace_id: "ws_1".into(),
                    tab_id: "tab_2".into(),
                    label: None,
                    cwd: None,
                    foreground_cwd: None,
                    focused: false,
                    right_click_passthrough: false,
                },
                ClientShellPane {
                    pane_id: "pane_3".into(),
                    workspace_id: "ws_1".into(),
                    tab_id: "tab_3".into(),
                    label: None,
                    cwd: None,
                    foreground_cwd: None,
                    focused: false,
                    right_click_passthrough: false,
                },
            ],
            agents: vec![
                make_agent("pane_1", "ws_1", "tab_1", AgentStatus::Working),
                make_agent("pane_2a", "ws_1", "tab_2", AgentStatus::Working),
                make_agent("pane_2b", "ws_1", "tab_2", AgentStatus::Blocked),
            ],
            commands: Vec::new(),
        }
    }

    #[test]
    fn test_indicator_prefix_width() {
        assert_eq!(tab_indicator_prefix_width(0, true), 0);
        assert_eq!(tab_indicator_prefix_width(0, false), 0);
        assert_eq!(tab_indicator_prefix_width(1, true), 2);
        assert_eq!(tab_indicator_prefix_width(1, false), 2);
        assert_eq!(tab_indicator_prefix_width(2, true), 4);
        assert_eq!(tab_indicator_prefix_width(2, false), 3);
        assert_eq!(tab_indicator_prefix_width(3, true), 6);
        assert_eq!(tab_indicator_prefix_width(3, false), 4);
    }

    #[test]
    fn test_tab_agent_statuses_defaults() {
        let snapshot = make_test_snapshot();
        let config = ClientShellConfig::from_config(&Config::default());

        assert_eq!(
            tab_agent_statuses("tab_1", &snapshot, &config),
            vec![AgentStatus::Working]
        );
        assert_eq!(
            tab_agent_statuses("tab_2", &snapshot, &config),
            vec![AgentStatus::Working, AgentStatus::Blocked]
        );
        assert_eq!(
            tab_agent_statuses("tab_3", &snapshot, &config),
            Vec::<AgentStatus>::new()
        );
    }

    #[test]
    fn test_tab_agent_statuses_master_switch() {
        let snapshot = make_test_snapshot();
        let mut cfg = Config::default();
        cfg.ui.tab_status = false;
        let config = ClientShellConfig::from_config(&cfg);

        assert!(tab_agent_statuses("tab_1", &snapshot, &config).is_empty());
        assert!(tab_agent_statuses("tab_2", &snapshot, &config).is_empty());
    }

    #[test]
    fn test_tab_agent_statuses_idle_filtering() {
        let mut snapshot = make_test_snapshot();
        snapshot
            .agents
            .push(make_agent("pane_3", "ws_1", "tab_3", AgentStatus::Idle));

        let default_config = ClientShellConfig::from_config(&Config::default());
        assert_eq!(
            tab_agent_statuses("tab_3", &snapshot, &default_config),
            vec![AgentStatus::Idle]
        );

        let mut cfg = Config::default();
        cfg.ui.tab_status_idle = false;
        let config_no_idle = ClientShellConfig::from_config(&cfg);
        assert!(tab_agent_statuses("tab_3", &snapshot, &config_no_idle).is_empty());
    }

    #[test]
    fn test_tab_agent_statuses_priority_order() {
        let snapshot = make_test_snapshot();
        let mut cfg = Config::default();
        cfg.ui.tab_status_order = TabStatusOrderConfig::Priority;
        let config = ClientShellConfig::from_config(&cfg);

        // Blocked has higher priority than Working
        assert_eq!(
            tab_agent_statuses("tab_2", &snapshot, &config),
            vec![AgentStatus::Blocked, AgentStatus::Working]
        );
    }

    #[test]
    fn test_tab_agent_statuses_max_capping() {
        let snapshot = make_test_snapshot();
        let mut cfg = Config::default();
        cfg.ui.tab_status_max = 1;
        let config_max_1 = ClientShellConfig::from_config(&cfg);

        assert_eq!(
            tab_agent_statuses("tab_2", &snapshot, &config_max_1),
            vec![AgentStatus::Working]
        );

        cfg.ui.tab_status_order = TabStatusOrderConfig::Priority;
        let config_priority_max_1 = ClientShellConfig::from_config(&cfg);
        assert_eq!(
            tab_agent_statuses("tab_2", &snapshot, &config_priority_max_1),
            vec![AgentStatus::Blocked]
        );
    }

    #[test]
    fn test_render_tab_bar_default() {
        let snapshot = make_test_snapshot();
        let config = ClientShellConfig::from_config(&Config::default());
        let area = Rect::new(0, 0, 40, 1);
        let mut buffer = Buffer::empty(area);
        let mut tab_scroll = 0;
        let mut reveal = false;
        let mut hits = ShellHitMap::default();

        render_tab_bar(
            &mut buffer,
            area,
            &snapshot,
            &config,
            &mut tab_scroll,
            &mut reveal,
            None,
            &mut hits,
        );

        assert_eq!(hits.tabs.len(), 3);
        let row_text: String = (0..area.width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();

        // Tab 1: focused, working: contains "● 1"
        // Tab 2: unfocused, working + blocked: contains "● ● 2"
        // Tab 3: unfocused, no agent: contains "3"
        assert!(row_text.contains("● 1"));
        assert!(row_text.contains("● ● 2"));
        assert!(row_text.contains("3"));

        // Check color of Tab 1 dot: yellow fg on accent bg
        let tab1_rect = hits.tabs[0].0;
        let dot_cell = (tab1_rect.x..tab1_rect.right())
            .map(|x| &buffer[(x, 0)])
            .find(|cell| cell.symbol() == "●")
            .unwrap();
        assert_eq!(dot_cell.style().fg, Some(config.palette.yellow));
        assert_eq!(dot_cell.style().bg, Some(config.palette.accent));

        // Check color of Tab 2 dots: yellow and red on surface0 bg
        let tab2_rect = hits.tabs[1].0;
        let tab2_dots: Vec<_> = (tab2_rect.x..tab2_rect.right())
            .map(|x| &buffer[(x, 0)])
            .filter(|cell| cell.symbol() == "●")
            .collect();
        assert_eq!(tab2_dots.len(), 2);
        assert_eq!(tab2_dots[0].style().fg, Some(config.palette.yellow));
        assert_eq!(tab2_dots[0].style().bg, Some(config.palette.surface0));
        assert_eq!(tab2_dots[1].style().fg, Some(config.palette.red));
        assert_eq!(tab2_dots[1].style().bg, Some(config.palette.surface0));
    }

    #[test]
    fn test_render_tab_bar_numbers_follow_status_indicators() {
        let mut snapshot = make_test_snapshot();
        snapshot.tabs[0].label = "main".into();
        snapshot.tabs[0].custom_label = true;
        snapshot.tabs[1].label = "logs".into();
        snapshot.tabs[1].custom_label = true;
        snapshot.tabs[1].number = 7;
        let mut cfg = Config::default();
        cfg.ui.tab_bar_numbers = true;
        let config = ClientShellConfig::from_config(&cfg);
        let area = Rect::new(0, 0, 50, 1);
        let mut buffer = Buffer::empty(area);
        let mut tab_scroll = 0;
        let mut reveal = false;
        let mut hits = ShellHitMap::default();

        render_tab_bar(
            &mut buffer,
            area,
            &snapshot,
            &config,
            &mut tab_scroll,
            &mut reveal,
            None,
            &mut hits,
        );

        let row_text: String = (0..area.width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();
        assert!(row_text.contains("● 1 main"));
        assert!(row_text.contains("● ● 2 logs"));

        let active_rect = hits.tabs[0].0;
        let active_number = (active_rect.x..active_rect.right())
            .map(|x| &buffer[(x, 0)])
            .find(|cell| cell.symbol() == "1")
            .expect("active tab number");
        let active_label = (active_rect.x..active_rect.right())
            .map(|x| &buffer[(x, 0)])
            .find(|cell| cell.symbol() == "m")
            .expect("active tab label");
        assert_eq!(active_number.style(), active_label.style());

        let inactive_rect = hits.tabs[1].0;
        let inactive_number = (inactive_rect.x..inactive_rect.right())
            .map(|x| &buffer[(x, 0)])
            .find(|cell| cell.symbol() == "2")
            .expect("inactive tab number");
        let inactive_label = (inactive_rect.x..inactive_rect.right())
            .map(|x| &buffer[(x, 0)])
            .find(|cell| cell.symbol() == "l")
            .expect("inactive tab label");
        assert_eq!(inactive_number.style(), inactive_label.style());
    }

    #[test]
    fn test_render_tab_bar_compact_spacing() {
        let snapshot = make_test_snapshot();
        let mut cfg = Config::default();
        cfg.ui.tab_status_spacing = false;
        let config = ClientShellConfig::from_config(&cfg);
        let area = Rect::new(0, 0, 40, 1);
        let mut buffer = Buffer::empty(area);
        let mut tab_scroll = 0;
        let mut reveal = false;
        let mut hits = ShellHitMap::default();

        render_tab_bar(
            &mut buffer,
            area,
            &snapshot,
            &config,
            &mut tab_scroll,
            &mut reveal,
            None,
            &mut hits,
        );

        let row_text: String = (0..area.width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();

        // Compact: "●● 2" (no space between dots)
        assert!(row_text.contains("●● 2"));
    }

    #[test]
    fn test_render_tab_bar_symbols_and_priority() {
        let snapshot = make_test_snapshot();
        let mut cfg = Config::default();
        cfg.ui.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
        cfg.ui.tab_status_order = TabStatusOrderConfig::Priority;
        let config = ClientShellConfig::from_config(&cfg);
        let area = Rect::new(0, 0, 40, 1);
        let mut buffer = Buffer::empty(area);
        let mut tab_scroll = 0;
        let mut reveal = false;
        let mut hits = ShellHitMap::default();

        render_tab_bar(
            &mut buffer,
            area,
            &snapshot,
            &config,
            &mut tab_scroll,
            &mut reveal,
            None,
            &mut hits,
        );

        let row_text: String = (0..area.width)
            .map(|x| buffer[(x, 0)].symbol().to_string())
            .collect();

        // Symbols: Working is ◐, Blocked is ×
        // Tab 1: "◐ 1"
        // Tab 2: Priority order -> Blocked then Working: "× ◐ 2"
        assert!(row_text.contains("◐ 1"));
        assert!(row_text.contains("× ◐ 2"));
    }

    #[test]
    fn trailing_scroll_limit_accounts_for_full_widths_and_separators() {
        for (widths, available, expected) in [
            (&[][..], 0, 0),
            (&[8, 13][..], 0, 1),
            (&[8, 13][..], 1, 1),
            (&[8, 13][..], 12, 1),
            (&[8, 13][..], 21, 1),
            (&[8, 13][..], 22, 0),
            (&[8, 13][..], 30, 0),
            (&[8, u16::MAX][..], u16::MAX, 1),
        ] {
            assert_eq!(
                max_tab_scroll(widths, available),
                expected,
                "widths={widths:?}, available={available}"
            );
        }
    }
}
