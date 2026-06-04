use hop_core::Asset;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use super::app::TuiApp;

pub fn draw(frame: &mut Frame<'_>, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Hop ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(env!("CARGO_PKG_VERSION")),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        chunks[0],
    );

    let prompt = if app.searching { "search" } else { "filter" };
    frame.render_widget(
        Paragraph::new(format!("{prompt}> {}", app.query))
            .block(Block::default().borders(Borders::ALL)),
        chunks[1],
    );

    let filtered = app.filtered_assets();
    let items: Vec<ListItem<'_>> = filtered
        .iter()
        .enumerate()
        .map(|(idx, asset)| asset_item(asset, idx == app.selected))
        .collect();
    frame.render_widget(
        List::new(items).block(Block::default().title("Assets").borders(Borders::ALL)),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new("Enter: connect  /: search  Up/Down: select  Ctrl+C: clear  q: quit")
            .block(Block::default().borders(Borders::ALL)),
        chunks[3],
    );
}

fn asset_item(asset: &Asset, selected: bool) -> ListItem<'_> {
    let marker = if selected { ">" } else { " " };
    let credential = if asset.credential_id.is_some() { "managed" } else { "proxy-only" };
    let tags = if asset.tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", asset.tags.join("]["))
    };
    ListItem::new(Line::from(vec![
        Span::styled(marker, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(asset.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("  {}:{}  {}{}", asset.hostname, asset.port, credential, tags)),
    ]))
}
