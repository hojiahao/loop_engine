use ratatui::{
    Frame,
    widgets::{Block, Borders, Paragraph},
};

pub fn draw_bootstrap(frame: &mut Frame<'_>) {
    let status = Paragraph::new("Control plane scaffold ready")
        .block(Block::default().title("Loop Engine").borders(Borders::ALL));
    frame.render_widget(status, frame.area());
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn bootstrap_view_renders_without_overflow() {
        let backend = TestBackend::new(48, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(draw_bootstrap).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Loop Engine"));
    }
}
