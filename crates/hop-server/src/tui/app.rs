use std::cell::RefCell;

use anyhow::Result;
use hop_core::Asset;
use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Config, Matcher,
};
use ratatui::{layout::Rect, TerminalOptions, Viewport};
use russh::{server::Handle, ChannelId};

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
    pub app: TuiApp,
    pub input: InputAdapter,
}

impl TuiResources {
    pub fn new(
        handle: Handle,
        channel_id: ChannelId,
        width: u16,
        height: u16,
        assets: Vec<Asset>,
    ) -> Result<Self> {
        let backend =
            ratatui::backend::CrosstermBackend::new(TerminalHandle::start(handle, channel_id));
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
            app: TuiApp::new(assets),
            input: InputAdapter::default(),
        })
    }

    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        self.terminal.resize(Rect {
            x: 0,
            y: 0,
            width: width.max(40),
            height: height.max(10),
        })?;
        self.render()
    }

    pub fn render(&mut self) -> Result<()> {
        self.terminal.draw(|frame| views::draw(frame, &self.app))?;
        Ok(())
    }

    pub fn handle_bytes(&mut self, bytes: &[u8]) -> Result<TuiAction> {
        let mut action = TuiAction::None;
        for input in self.input.parse(bytes) {
            action = self.app.handle_input(input);
            if !matches!(action, TuiAction::None) {
                break;
            }
        }
        self.render()?;
        Ok(action)
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

    pub fn selected_asset(&self) -> Option<Asset> {
        let items = self.filtered_assets();
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn enter_returns_selected_asset() {
        let mut app = TuiApp::new(vec![asset("web-prod-01")]);
        let action = app.handle_input(TuiInput::Enter);
        match action {
            TuiAction::Connect(asset) => assert_eq!(asset.name, "web-prod-01"),
            _ => panic!("expected connect"),
        }
    }
}
