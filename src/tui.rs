//! Live `status --watch` view.

use crate::config::Config;
use crate::status::{gather, render_lines};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(2);

pub fn run(cfg: &Config) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, cfg);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, cfg: &Config) -> Result<()> {
    let mut lines = render_lines(&gather(cfg));
    let mut last = Instant::now();
    loop {
        terminal.draw(|f| {
            let text = lines.join("\n");
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" mediactl status (shadow, read-only) - q to quit ");
            f.render_widget(Paragraph::new(text).block(block), f.area());
        })?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                let quit = matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)
                    || (k.code == KeyCode::Char('c')
                        && k.modifiers.contains(KeyModifiers::CONTROL));
                if quit {
                    return Ok(());
                }
            }
        }
        if last.elapsed() >= REFRESH {
            lines = render_lines(&gather(cfg));
            last = Instant::now();
        }
    }
}
