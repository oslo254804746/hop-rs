use hop_core::Asset;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::app::{AssetListItem, TuiApp};

pub fn draw(frame: &mut Frame<'_>, app: &TuiApp) {
    let status_height = if app.searching || !app.query.is_empty() {
        1
    } else {
        0
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(status_height),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, chunks[0], app, chunks[3].height);
    draw_separator(frame, chunks[1]);
    if status_height > 0 {
        draw_status(frame, chunks[2], app);
    }
    draw_assets(frame, chunks[3], app);
    draw_separator(frame, chunks[4]);
    draw_help(frame, chunks[5], app);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &TuiApp, list_height: u16) {
    let filtered = app.filtered_assets();
    let count = filtered.len();
    let count_text = if count == 1 {
        "1 asset".to_string()
    } else {
        format!("{count} assets")
    };
    let visible_count = visible_asset_count(list_height);
    let right_text = if visible_count > 0 && count > visible_count {
        let selected = app.selected.min(count.saturating_sub(1)) + 1;
        format!("{count_text}  {selected}/{count}")
    } else {
        count_text
    };

    let title_len = 5 + env!("CARGO_PKG_VERSION").len(); // "Hop " + version
    let padding = area
        .width
        .saturating_sub(title_len as u16 + display_width(&right_text) as u16 + 4);

    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Hop",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" ".repeat(padding as usize)),
        Span::styled(right_text, Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_separator(frame: &mut Frame<'_>, area: Rect) {
    let line = Line::from(Span::styled(
        " ".to_string() + &"─".repeat(area.width.saturating_sub(2) as usize) + " ",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_status(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let available = area.width.saturating_sub(4) as usize;
    let content = truncate_to_width(&app.query, available);
    let line = if app.searching {
        Line::from(vec![
            Span::raw("  "),
            Span::styled("›", Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::raw(content),
        ])
    } else {
        Line::from(vec![
            Span::raw("  "),
            Span::styled("filter", Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::raw(content),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_assets(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let filtered = app.filtered_assets();
    let grouped = app.grouped_items();
    let available_lines = area.height as usize;
    let visible_count = visible_asset_count(area.height);

    if filtered.is_empty() {
        draw_empty_assets(frame, area, app);
        return;
    }
    if visible_count == 0 {
        return;
    }

    let selected = app.selected.min(filtered.len().saturating_sub(1));

    let scroll_offset = if selected >= visible_count {
        selected - visible_count + 1
    } else {
        0
    };

    let mut lines: Vec<Line<'_>> = Vec::new();
    let mut asset_idx = 0usize;
    for item in grouped {
        match item {
            AssetListItem::Group(label) => {
                if asset_idx >= scroll_offset && lines.len() < available_lines {
                    lines.push(group_line(&label, area.width));
                }
            }
            AssetListItem::Asset(asset) => {
                let actual_idx = asset_idx;
                asset_idx += 1;
                if actual_idx < scroll_offset {
                    continue;
                }
                if lines.len() >= available_lines {
                    break;
                }
                let is_selected = actual_idx == selected;
                let (line1, line2) = asset_lines(&asset, is_selected, area.width);
                lines.push(line1);
                if lines.len() < available_lines {
                    lines.push(line2);
                }
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn group_line(label: &str, width: u16) -> Line<'static> {
    let content = format!(
        "  {}",
        truncate_to_width(label, width.saturating_sub(2) as usize)
    );
    Line::from(Span::styled(
        content,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn draw_empty_assets(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let message = if app.query.is_empty() {
        "No assets configured".to_string()
    } else {
        format!("No assets match \"{}\"", app.query)
    };
    let content = truncate_to_width(&message, area.width.saturating_sub(4) as usize);
    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(content, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn asset_lines(asset: &Asset, selected: bool, width: u16) -> (Line<'static>, Line<'static>) {
    let marker = if selected { "▸" } else { " " };
    let credential = if asset.credential_id.is_some() {
        "managed"
    } else {
        "proxy-only"
    };
    let addr = format!("{}:{}", asset.hostname, asset.port);
    let tags = if asset.tags.is_empty() {
        String::new()
    } else {
        format!("[{}]", asset.tags.join("]["))
    };

    let bg = if selected {
        Style::default().bg(Color::DarkGray)
    } else {
        Style::default()
    };

    // Line 1: "  ▸ name                    credential"
    let line1_width = width as usize;
    let prefix_width = 4;
    let right_margin = 2;
    let right = truncate_to_width(
        credential,
        line1_width
            .saturating_sub(prefix_width)
            .saturating_sub(right_margin),
    );
    let right_width = display_width(&right) + right_margin;
    let name = truncate_to_width(
        &asset.name,
        line1_width.saturating_sub(prefix_width + right_width),
    );
    let gap1 = line1_width.saturating_sub(prefix_width + display_width(&name) + right_width);

    let marker_style = if selected {
        bg.fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let name_style = if selected {
        bg.fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        bg.add_modifier(Modifier::BOLD)
    };
    let cred_style = bg;

    let line1 = Line::from(vec![
        Span::styled("  ", bg),
        Span::styled(marker, marker_style),
        Span::styled(" ", bg),
        Span::styled(name, name_style),
        Span::styled(" ".repeat(gap1), bg),
        Span::styled(right, cred_style),
        Span::styled("  ", bg),
    ]);

    // Line 2: "    host:port                     [tags]"
    let line2_width = width as usize;
    let right = truncate_to_width(
        &tags,
        line2_width
            .saturating_sub(prefix_width)
            .saturating_sub(right_margin),
    );
    let right_width = display_width(&right) + right_margin;
    let addr = truncate_to_width(
        &addr,
        line2_width.saturating_sub(prefix_width + right_width),
    );
    let gap2 = line2_width.saturating_sub(prefix_width + display_width(&addr) + right_width);

    let addr_style = if selected {
        bg.fg(Color::White)
    } else {
        bg.fg(Color::DarkGray)
    };
    let tags_style = bg.fg(Color::Blue);

    let line2 = Line::from(vec![
        Span::styled("    ", bg),
        Span::styled(addr, addr_style),
        Span::styled(" ".repeat(gap2), bg),
        Span::styled(right, tags_style),
        Span::styled("  ", bg),
    ]);

    (line1, line2)
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, app: &TuiApp) {
    let key_style = Style::default().fg(Color::Cyan);
    let desc_style = Style::default().fg(Color::DarkGray);

    let mut spans = vec![
        Span::raw("  "),
        Span::styled("Enter", key_style),
        Span::styled(" connect  ", desc_style),
        Span::styled("/", key_style),
        Span::styled(" search  ", desc_style),
        Span::styled("↑↓", key_style),
        Span::styled(" select  ", desc_style),
    ];
    if app.searching {
        spans.push(Span::styled("Esc", key_style));
        spans.push(Span::styled(" close  ", desc_style));
    }
    if app.searching || !app.query.is_empty() {
        spans.push(Span::styled("Ctrl+C", key_style));
        spans.push(Span::styled(" clear  ", desc_style));
    }
    spans.push(Span::styled("q", key_style));
    spans.push(Span::styled(" quit", desc_style));

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn visible_asset_count(list_height: u16) -> usize {
    (list_height as usize) / 2
}

fn truncate_to_width(value: &str, max_width: usize) -> String {
    if display_width(value) <= max_width {
        return value.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut output = String::new();
    let mut width = 0usize;
    for ch in value.chars() {
        let mut buf = [0; 4];
        let ch_width = display_width(ch.encode_utf8(&mut buf));
        if width + ch_width > max_width - 1 {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output.push('…');
    output
}

fn display_width(value: &str) -> usize {
    Span::raw(value).width()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn asset(name: &str) -> Asset {
        Asset {
            id: name.to_string(),
            name: name.to_string(),
            hostname: "127.0.0.1".to_string(),
            port: 22,
            description: None,
            tags: vec![],
            credential_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn buffer_text(app: &TuiApp, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn active_filter_remains_visible_after_search_closes() {
        let mut app = TuiApp::new(vec![asset("web-prod-01"), asset("db-prod-01")]);
        app.query = "web".to_string();
        app.searching = false;

        let output = buffer_text(&app, 80, 12);

        assert!(output.contains("filter"));
        assert!(output.contains("web"));
    }

    #[test]
    fn empty_filtered_list_explains_that_no_assets_match() {
        let mut app = TuiApp::new(vec![asset("web-prod-01")]);
        app.query = "missing".to_string();

        let output = buffer_text(&app, 80, 12);

        assert!(output.contains("No assets match"));
        assert!(output.contains("missing"));
    }

    #[test]
    fn asset_lines_fit_within_available_width() {
        let mut long = asset("very-long-production-hostname-that-would-overlap-status");
        long.hostname = "very-long-hostname-for-a-private-network.example.internal".to_string();
        long.tags = vec![
            "production".to_string(),
            "critical".to_string(),
            "database".to_string(),
        ];
        long.credential_id = Some("managed".to_string());

        let (line1, line2) = asset_lines(&long, true, 40);

        assert!(line1.width() <= 40, "line1 width was {}", line1.width());
        assert!(line2.width() <= 40, "line2 width was {}", line2.width());
    }

    #[test]
    fn long_search_query_fits_status_line_width() {
        let mut app = TuiApp::new(vec![asset("web-prod-01")]);
        app.searching = true;
        app.query = "very-long-query-that-should-not-overflow-the-status-row".to_string();

        let output = buffer_text(&app, 40, 12);

        assert!(output.contains("very-long-query-that"));
        assert!(output.contains("…"));
    }

    #[test]
    fn list_shows_scroll_position_when_not_all_assets_fit() {
        let mut app = TuiApp::new((0..8).map(|idx| asset(&format!("asset-{idx}"))).collect());
        app.selected = 6;

        let output = buffer_text(&app, 80, 10);

        assert!(output.contains("7/8"));
    }

    #[test]
    fn help_reflects_search_and_filter_state() {
        let mut app = TuiApp::new(vec![asset("web-prod-01")]);
        app.searching = true;
        app.query = "web".to_string();

        let output = buffer_text(&app, 80, 12);

        assert!(output.contains("Esc"));
        assert!(output.contains("close"));
        assert!(output.contains("Ctrl+C"));
        assert!(output.contains("clear"));
    }
}
