use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use super::text::display_width;
use super::widgets::panel_contrast_fg;
use crate::{
    app::state::{CopyFeedback, Palette},
    config::ToastClipboardPosition,
};

pub(crate) fn copy_feedback_rect(
    area: Rect,
    feedback: &CopyFeedback,
    offset_rows: u16,
    position: ToastClipboardPosition,
) -> Rect {
    if area.is_empty() {
        return Rect::default();
    }

    let content_width = feedback.message.len() as u16 + 4;
    let width = content_width.min(area.width);
    let height = 3u16.min(area.height);
    let x = match position {
        ToastClipboardPosition::TopLeft | ToastClipboardPosition::BottomLeft => area.x,
        ToastClipboardPosition::TopCenter | ToastClipboardPosition::BottomCenter => {
            area.x + area.width.saturating_sub(width) / 2
        }
        ToastClipboardPosition::TopRight | ToastClipboardPosition::BottomRight => {
            area.x + area.width.saturating_sub(width)
        }
    };
    let y = match position {
        ToastClipboardPosition::TopLeft
        | ToastClipboardPosition::TopCenter
        | ToastClipboardPosition::TopRight => area.y + offset_rows.min(area.height),
        ToastClipboardPosition::BottomLeft
        | ToastClipboardPosition::BottomCenter
        | ToastClipboardPosition::BottomRight => {
            area.y + area.height.saturating_sub(height + offset_rows)
        }
    };
    Rect::new(x, y, width, height)
}

pub(crate) fn render_copy_feedback_buffer(
    buffer: &mut Buffer,
    area: Rect,
    feedback: &CopyFeedback,
    offset_rows: u16,
    position: ToastClipboardPosition,
    palette: &Palette,
) -> Rect {
    let feedback_area = copy_feedback_rect(area, feedback, offset_rows, position);
    if feedback_area.is_empty() {
        return feedback_area;
    }

    Clear.render(feedback_area, buffer);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.green))
        .style(Style::default().bg(palette.panel_bg));
    let inner = block.inner(feedback_area);
    block.render(feedback_area, buffer);

    if inner.height == 0 {
        return feedback_area;
    }

    let text = Line::from(vec![
        Span::styled("●", Style::default().fg(palette.green).bg(palette.panel_bg)),
        Span::raw(" "),
        Span::styled(
            &feedback.message,
            Style::default()
                .fg(palette.text)
                .bg(palette.panel_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    Paragraph::new(text).render(inner, buffer);
    feedback_area
}

pub(crate) const CONFIG_DIAGNOSTIC_DISMISS_LABEL: &str = "✕";

pub(crate) fn render_config_diagnostic_buffer(
    buffer: &mut Buffer,
    area: Rect,
    message: &str,
    palette: &Palette,
    mut covered: impl FnMut(Rect),
) -> (u16, Rect) {
    let style = Style::default()
        .fg(panel_contrast_fg(palette))
        .bg(palette.yellow)
        .add_modifier(Modifier::BOLD);
    let mut rendered_rows = 0u16;
    let mut dismiss = Rect::default();

    for (row, line) in message
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(area.height as usize)
        .enumerate()
    {
        let text = if row == 0 {
            format!(" {line}  {CONFIG_DIAGNOSTIC_DISMISS_LABEL} ")
        } else {
            format!(" {line} ")
        };
        let full_width = display_width(&text).min(u16::MAX as usize) as u16;
        let width = full_width.min(area.width);
        let diagnostic_area = Rect::new(
            area.x + area.width.saturating_sub(width),
            area.y + row as u16,
            width,
            1,
        );

        covered(diagnostic_area);
        Clear.render(diagnostic_area, buffer);
        Paragraph::new(Span::styled(text, style)).render(diagnostic_area, buffer);
        if row == 0 && full_width <= area.width && width >= 2 {
            dismiss = Rect::new(
                diagnostic_area.x + width.saturating_sub(2),
                diagnostic_area.y,
                1,
                1,
            );
        }
        rendered_rows = rendered_rows.saturating_add(1);
    }

    (rendered_rows, dismiss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_diagnostic_reports_dismiss_cell_on_first_row() {
        let palette = Palette::catppuccin();
        let area = Rect::new(0, 0, 106, 30);
        let mut buffer = Buffer::empty(area);
        let (rows, dismiss) =
            render_config_diagnostic_buffer(&mut buffer, area, "warn", &palette, |_| {});
        assert_eq!(rows, 1);
        // " warn  ✕ " is 9 cells wide, so the cross sits at the second-to-last cell.
        assert_eq!(dismiss, Rect::new(104, 0, 1, 1));
        assert_eq!(buffer[(104, 0)].symbol(), CONFIG_DIAGNOSTIC_DISMISS_LABEL);
    }

    #[test]
    fn config_diagnostic_has_no_dismiss_cell_when_truncated() {
        let palette = Palette::catppuccin();
        let area = Rect::new(0, 0, 5, 30);
        let mut buffer = Buffer::empty(area);
        let (rows, dismiss) =
            render_config_diagnostic_buffer(&mut buffer, area, "warn", &palette, |_| {});
        assert_eq!(rows, 1);
        assert!(dismiss.is_empty());
    }

    #[test]
    fn copy_feedback_rect_uses_configured_position() {
        let area = Rect::new(10, 20, 100, 40);
        let feedback = CopyFeedback {
            message: "copied to clipboard".to_owned(),
        };

        let top = copy_feedback_rect(area, &feedback, 0, ToastClipboardPosition::TopCenter);
        assert_eq!(top.y, area.y);
        assert_eq!(top.x, area.x + area.width.saturating_sub(top.width) / 2);

        let bottom = copy_feedback_rect(area, &feedback, 0, ToastClipboardPosition::BottomCenter);
        assert_eq!(bottom.bottom(), area.bottom());
        assert_eq!(
            bottom.x,
            area.x + area.width.saturating_sub(bottom.width) / 2
        );
    }
}
