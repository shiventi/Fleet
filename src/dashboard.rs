//! Shows configured agents in a terminal table, without polling or controlling them.

use crate::controller::ClusterSpec;
use crate::tls::TlsFiles;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Row, Table};
use std::io;

const ACCENT: Color = Color::Rgb(50, 108, 229);

/// Shows configured nodes without connecting to them yet.
/// q, Esc, or Ctrl+C closes the screen and restores the terminal.
/// TLS paths are reserved for future live checks and are not read yet.
///
/// # Errors
/// Returns an error if drawing or reading keyboard input fails.
///
/// # Panics
/// Panics if Ratatui cannot set up the terminal. Run in an interactive terminal.
pub fn run(cluster: &ClusterSpec, _files: &TlsFiles) -> io::Result<()> {
    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| draw(frame, cluster))?;

            if let Event::Key(key) = event::read()? {
                let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL));
                if key.kind == KeyEventKind::Press && quit {
                    break;
                }
            }
        }
        Ok(())
    })
}

/// Draws the title and agent table, hiding addresses on narrow terminals.
fn draw(frame: &mut Frame, cluster: &ClusterSpec) {
    let areas = Layout::vertical([Constraint::Length(3), Constraint::Min(0)])
        .margin(1)
        .split(frame.area());
    let heading = Layout::horizontal([Constraint::Min(0), Constraint::Length(12)]).split(areas[0]);
    let title = Paragraph::new("Fleet").style(Style::default().fg(ACCENT));
    let count = Paragraph::new(format!("Nodes: {}", cluster.nodes.len()))
        .alignment(Alignment::Right)
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(title, heading[0]);
    frame.render_widget(count, heading[1]);

    let compact = areas[1].width < 60;
    let rows = cluster.nodes.iter().map(|node| {
        let cells = if compact {
            vec![node.name.as_str(), "Not checked"]
        } else {
            vec![node.name.as_str(), node.address.as_str(), "Not checked"]
        };
        Row::new(cells)
    });
    let (labels, widths) = if compact {
        (
            vec!["Node", "Connection"],
            vec![Constraint::Percentage(50); 2],
        )
    } else {
        (
            vec!["Node", "Address", "Connection"],
            vec![
                Constraint::Percentage(25),
                Constraint::Percentage(45),
                Constraint::Percentage(30),
            ],
        )
    };
    let header = Row::new(labels)
        .style(Style::default().fg(Color::White).bg(ACCENT))
        .bottom_margin(1);
    let block = Block::default()
        .title(" Agents ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::horizontal(1));
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(2)
        .block(block);
    frame.render_widget(table, areas[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn draws_nodes_in_normal_and_compact_terminals() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster.json"))
            .expect("example cluster should parse");

        for width in [80, 40] {
            let mut terminal = Terminal::new(TestBackend::new(width, 15))
                .expect("test terminal should initialize");
            terminal
                .draw(|frame| draw(frame, &cluster))
                .expect("screen should draw");
            let screen: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();

            assert!(screen.contains("Fleet"));
            assert_eq!(terminal.backend().buffer()[(1, 1)].fg, ACCENT);
            assert!(screen.contains("Nodes: 2"));
            assert!(screen.contains("local-one"));
            assert!(screen.contains("local-two"));
            assert!(screen.contains("Not checked"));
            assert_eq!(screen.contains("127.0.0.1:7070"), width >= 60);
        }
    }
}
