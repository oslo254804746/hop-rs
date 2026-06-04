use std::cell::RefCell;
use std::io::Write;

use anyhow::Result;
use hop_core::Asset;
use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Config, Matcher,
};
use ratatui::{
    crossterm::{
        cursor::{Hide, MoveTo, Show},
        execute,
        style::ResetColor,
        terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::Rect,
    TerminalOptions, Viewport,
};

use super::{
    backend::{SshTerminal, TerminalHandle},
    input::{InputAdapter, TuiInput},
    views,
};

pub enum TuiAction {
    None,
    Quit,
    Connect(Asset),
}

pub struct TuiResources {
    pub terminal: SshTerminal,
    output: TerminalHandle,
    pub app: TuiApp,
    pub input: InputAdapter,
}

impl TuiResources {
    pub fn new(width: u16, height: u16, assets: Vec<Asset>) -> Result<Self> {
        Self::from_terminal_handle(TerminalHandle::new(), width, height, assets)
    }

    fn from_terminal_handle(
        handle: TerminalHandle,
        width: u16,
        height: u16,
        assets: Vec<Asset>,
    ) -> Result<Self> {
        let output = handle.clone();
        let backend = ratatui::backend::CrosstermBackend::new(handle);
        let terminal = ratatui::Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect {
                    x: 0,
                    y: 0,
                    width: width.max(40),
                    height: height.max(10),
                }),
            },
        )?;
        Ok(Self {
            terminal,
            output,
            app: TuiApp::new(assets),
            input: InputAdapter::default(),
        })
    }

    pub fn enter_screen(&mut self) -> Result<Vec<u8>> {
        execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            Hide,
            ResetColor,
            Clear(ClearType::All),
            MoveTo(0, 0)
        )?;
        self.terminal.clear()?;
        self.render()
    }

    pub fn leave_screen(&mut self) -> Result<Vec<u8>> {
        execute!(
            self.terminal.backend_mut(),
            Clear(ClearType::All),
            MoveTo(0, 0),
            ResetColor,
            Show,
            LeaveAlternateScreen
        )?;
        self.terminal.backend_mut().flush()?;
        Ok(self.take_output()?)
    }

    pub fn resume_after_target(&mut self) -> Result<Vec<u8>> {
        self.enter_screen()
    }

    pub fn resize(&mut self, width: u16, height: u16) -> Result<Vec<u8>> {
        self.terminal.resize(Rect {
            x: 0,
            y: 0,
            width: width.max(40),
            height: height.max(10),
        })?;
        self.render()
    }

    pub fn render(&mut self) -> Result<Vec<u8>> {
        self.terminal.draw(|frame| views::draw(frame, &self.app))?;
        Ok(self.take_output()?)
    }

    pub fn handle_bytes(&mut self, bytes: &[u8]) -> Result<(TuiAction, Vec<u8>)> {
        let mut action = TuiAction::None;
        for input in self.input.parse(bytes) {
            action = self.app.handle_input(input);
            if !matches!(action, TuiAction::None) {
                break;
            }
        }
        let output = self.render()?;
        Ok((action, output))
    }

    fn take_output(&mut self) -> std::io::Result<Vec<u8>> {
        self.terminal.backend_mut().flush()?;
        Ok(self.output.take_output())
    }
}

#[derive(Clone)]
pub struct TuiApp {
    assets: Vec<Asset>,
    filtered_cache: RefCell<FilteredAssetCache>,
    pub query: String,
    pub selected: usize,
    pub searching: bool,
}

#[derive(Clone, Debug)]
pub enum AssetListItem {
    Group(String),
    Asset(Asset),
}

#[derive(Clone)]
struct FilteredAssetCache {
    query: String,
    assets: Vec<Asset>,
}

impl TuiApp {
    pub fn new(assets: Vec<Asset>) -> Self {
        let filtered_assets = assets.clone();
        Self {
            assets,
            filtered_cache: RefCell::new(FilteredAssetCache {
                query: String::new(),
                assets: filtered_assets,
            }),
            query: String::new(),
            selected: 0,
            searching: false,
        }
    }

    pub fn filtered_assets(&self) -> Vec<Asset> {
        if self.filtered_cache.borrow().query == self.query {
            return self.filtered_cache.borrow().assets.clone();
        }

        let assets = self.compute_filtered_assets();
        *self.filtered_cache.borrow_mut() = FilteredAssetCache {
            query: self.query.clone(),
            assets: assets.clone(),
        };
        assets
    }

    fn compute_filtered_assets(&self) -> Vec<Asset> {
        if self.query.trim().is_empty() {
            return self.assets.clone();
        }
        if let Some(tag) = tag_query(&self.query) {
            return self
                .assets
                .iter()
                .filter(|asset| {
                    asset
                        .tags
                        .iter()
                        .any(|asset_tag| asset_tag.eq_ignore_ascii_case(tag))
                })
                .cloned()
                .collect();
        }
        let names: Vec<String> = self.assets.iter().map(|asset| asset.name.clone()).collect();
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let matched = pattern.match_list(names, &mut matcher);
        matched
            .into_iter()
            .filter_map(|(name, _)| self.assets.iter().find(|asset| asset.name == name).cloned())
            .collect()
    }

    fn ordered_filtered_assets(&self) -> Vec<Asset> {
        let mut assets = self.filtered_assets();
        assets.sort_by(|left, right| {
            asset_group_label(left)
                .cmp(&asset_group_label(right))
                .then_with(|| left.name.cmp(&right.name))
        });
        assets
    }

    pub fn grouped_items(&self) -> Vec<AssetListItem> {
        let mut items = Vec::new();
        let mut current_group = String::new();
        for asset in self.ordered_filtered_assets() {
            let group = asset_group_label(&asset);
            if group != current_group {
                current_group = group.clone();
                items.push(AssetListItem::Group(group));
            }
            items.push(AssetListItem::Asset(asset));
        }
        items
    }

    pub fn selected_asset(&self) -> Option<Asset> {
        let items = self.ordered_filtered_assets();
        items
            .get(self.selected.min(items.len().saturating_sub(1)))
            .cloned()
    }

    pub fn handle_input(&mut self, input: TuiInput) -> TuiAction {
        match input {
            TuiInput::Char('q') if !self.searching => return TuiAction::Quit,
            TuiInput::Char('/') if !self.searching => {
                self.searching = true;
                self.query.clear();
                self.selected = 0;
            }
            TuiInput::Char(ch) if self.searching => {
                self.query.push(ch);
                self.selected = 0;
            }
            TuiInput::Backspace if self.searching => {
                self.query.pop();
                self.selected = 0;
            }
            TuiInput::Escape => {
                self.searching = false;
            }
            TuiInput::CtrlC => {
                self.query.clear();
                self.searching = false;
                self.selected = 0;
            }
            TuiInput::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            TuiInput::Down => {
                let len = self.filtered_assets().len();
                if len > 0 {
                    self.selected = (self.selected + 1).min(len - 1);
                }
            }
            TuiInput::Enter => {
                if let Some(asset) = self.selected_asset() {
                    return TuiAction::Connect(asset);
                }
            }
            _ => {}
        }
        TuiAction::None
    }
}

fn tag_query(query: &str) -> Option<&str> {
    let query = query.trim();
    let (prefix, value) = query.split_once(':')?;
    prefix
        .eq_ignore_ascii_case("tag")
        .then(|| value.trim())
        .filter(|value| !value.is_empty())
}

fn asset_group_label(asset: &Asset) -> String {
    asset
        .tags
        .first()
        .cloned()
        .unwrap_or_else(|| "untagged".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        tagged_asset(name, &[])
    }

    fn tagged_asset(name: &str, tags: &[&str]) -> Asset {
        Asset {
            id: name.to_string(),
            name: name.to_string(),
            hostname: "127.0.0.1".to_string(),
            port: 22,
            description: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            credential_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn test_tui() -> TuiResources {
        let handle = TerminalHandle::new_for_test();
        TuiResources::from_terminal_handle(handle, 80, 24, vec![asset("web-prod-01")]).unwrap()
    }

    #[test]
    fn enter_screen_switches_to_clean_alternate_screen() {
        let mut tui = test_tui();

        let output = tui.enter_screen().unwrap();

        assert!(output
            .windows(b"\x1b[?1049h".len())
            .any(|w| w == b"\x1b[?1049h"));
        assert!(output.windows(b"\x1b[2J".len()).any(|w| w == b"\x1b[2J"));
        assert!(String::from_utf8_lossy(&output).contains("web-prod-01"));
    }

    #[test]
    fn leave_screen_restores_main_screen_for_target_session() {
        let mut tui = test_tui();
        tui.enter_screen().unwrap();

        let output = tui.leave_screen().unwrap();

        assert!(output
            .windows(b"\x1b[?25h".len())
            .any(|w| w == b"\x1b[?25h"));
        assert!(output.windows(b"\x1b[2J".len()).any(|w| w == b"\x1b[2J"));
        assert!(
            output.windows(b"\x1b[H".len()).any(|w| w == b"\x1b[H")
                || output
                    .windows(b"\x1b[1;1H".len())
                    .any(|w| w == b"\x1b[1;1H")
        );
        assert!(output
            .windows(b"\x1b[?1049l".len())
            .any(|w| w == b"\x1b[?1049l"));
    }

    #[test]
    fn resume_after_target_forces_full_redraw() {
        let mut tui = test_tui();
        tui.enter_screen().unwrap();

        let output = tui.resume_after_target().unwrap();

        assert!(output
            .windows(b"\x1b[?1049h".len())
            .any(|w| w == b"\x1b[?1049h"));
        assert!(output.windows(b"\x1b[2J".len()).any(|w| w == b"\x1b[2J"));
        assert!(String::from_utf8_lossy(&output).contains("web-prod-01"));
    }

    #[test]
    fn fuzzy_filter_selects_assets() {
        let mut app = TuiApp::new(vec![asset("web-prod-01"), asset("db-prod-01")]);
        app.query = "web".to_string();
        assert_eq!(app.filtered_assets()[0].name, "web-prod-01");
    }

    #[test]
    fn cached_filter_refreshes_when_query_changes() {
        let mut app = TuiApp::new(vec![asset("web-prod-01"), asset("db-prod-01")]);

        assert_eq!(app.filtered_assets().len(), 2);
        app.query = "db".to_string();
        assert_eq!(app.filtered_assets()[0].name, "db-prod-01");
    }

    #[test]
    fn tag_query_filters_assets_by_tag() {
        let mut app = TuiApp::new(vec![
            tagged_asset("web-prod-01", &["prod", "web"]),
            tagged_asset("db-dev-01", &["dev", "db"]),
        ]);

        app.query = "tag:prod".to_string();

        assert_eq!(app.filtered_assets().len(), 1);
        assert_eq!(app.filtered_assets()[0].name, "web-prod-01");
    }

    #[test]
    fn grouped_items_include_tag_headers() {
        let app = TuiApp::new(vec![
            tagged_asset("web-prod-01", &["prod", "web"]),
            tagged_asset("db-dev-01", &["dev", "db"]),
        ]);

        let groups = app.grouped_items();

        assert!(groups
            .iter()
            .any(|item| matches!(item, AssetListItem::Group(label) if label == "dev")));
        assert!(groups
            .iter()
            .any(|item| matches!(item, AssetListItem::Group(label) if label == "prod")));
    }

    #[test]
    fn selected_asset_follows_grouped_display_order() {
        let app = TuiApp::new(vec![
            tagged_asset("z-last", &["prod"]),
            tagged_asset("a-first", &["dev"]),
        ]);

        assert_eq!(app.selected_asset().unwrap().name, "a-first");
    }

    #[test]
    fn enter_returns_selected_asset() {
        let mut app = TuiApp::new(vec![asset("web-prod-01")]);
        let action = app.handle_input(TuiInput::Enter);
        match action {
            TuiAction::Connect(asset) => assert_eq!(asset.name, "web-prod-01"),
            _ => panic!("expected connect"),
        }
    }
}
