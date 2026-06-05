use termwiz::input::{InputEvent, InputParser, KeyCode, Modifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiInput {
    Char(char),
    Enter,
    Backspace,
    Escape,
    Up,
    Down,
    CtrlC,
}

#[derive(Debug)]
pub struct InputAdapter {
    parser: InputParser,
}

impl Default for InputAdapter {
    fn default() -> Self {
        Self {
            parser: InputParser::new(),
        }
    }
}

impl InputAdapter {
    pub fn parse(&mut self, bytes: &[u8]) -> Vec<TuiInput> {
        let events = self.parser.parse_as_vec(bytes, false);
        events.into_iter().filter_map(map_event).collect()
    }
}

fn map_event(event: InputEvent) -> Option<TuiInput> {
    match event {
        InputEvent::Key(key) => match key.key {
            KeyCode::Char('c') | KeyCode::Char('C') if key.modifiers.contains(Modifiers::CTRL) => {
                Some(TuiInput::CtrlC)
            }
            KeyCode::Char(ch) if key.modifiers.is_empty() || key.modifiers == Modifiers::SHIFT => {
                Some(TuiInput::Char(ch))
            }
            KeyCode::Enter => Some(TuiInput::Enter),
            KeyCode::Char('\r') | KeyCode::Char('\n') => Some(TuiInput::Enter),
            KeyCode::Backspace | KeyCode::Char('\u{8}') | KeyCode::Char('\u{7f}') => {
                Some(TuiInput::Backspace)
            }
            KeyCode::Escape => Some(TuiInput::Escape),
            KeyCode::UpArrow | KeyCode::ApplicationUpArrow => Some(TuiInput::Up),
            KeyCode::DownArrow | KeyCode::ApplicationDownArrow => Some(TuiInput::Down),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_tui_keys() {
        let mut input = InputAdapter::default();
        assert_eq!(input.parse(b"/"), vec![TuiInput::Char('/')]);
        assert_eq!(input.parse(b"\r"), vec![TuiInput::Enter]);
        assert_eq!(input.parse(b"\x03"), vec![TuiInput::CtrlC]);
        assert_eq!(input.parse(b"\x1b[A"), vec![TuiInput::Up]);
    }
}
