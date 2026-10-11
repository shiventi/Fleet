//! Collects agent status in the background and shows a terminal table.

use crate::client::FleetClient;
use crate::controller::ClusterSpec;
use crate::protocol::StatusResponse;
use crate::tls::TlsFiles;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Row, Table};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

const ACCENT: Color = Color::Rgb(50, 108, 229);

/// Shows configured nodes while collecting their status in the background.
/// q, Esc, or Ctrl+C closes the screen and restores the terminal.
/// Rows show the latest reply or connection error, not application health.
/// Checks run one at a time, with a two-second pause after each round.
/// Credentials are loaded once. Quitting waits for the current check or pause
/// and leaves agents running.
///
/// # Errors
/// Returns an error if credentials cannot be loaded or terminal I/O fails.
///
/// # Panics
/// Panics if Ratatui cannot set up the terminal. Run in an interactive terminal.
pub fn run(cluster: &ClusterSpec, files: &TlsFiles) -> io::Result<()> {
    let client = FleetClient::new(files)?;
    let mut states: Vec<NodeState> = Vec::new();
    for _ in &cluster.nodes {
        states.push(NodeState::NotChecked);
    }

    let (sender, receiver) = mpsc::channel::<(usize, NodeState)>();
    let keep_alive = AtomicBool::new(true);

    std::thread::scope(|scope| {
        scope.spawn(|| poll_nodes(cluster, &client, sender, &keep_alive));
        let result = ratatui::run(|terminal| {
            loop {
                for (index, state) in receiver.try_iter() {
                    states[index] = state;
                }
                terminal.draw(|frame| draw(frame, cluster, &states))?;

                if event::poll(Duration::from_millis(100))? {
                    if let Event::Key(key) = event::read()? {
                        let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL));
                        if key.kind == KeyEventKind::Press && quit {
                            break;
                        }
                    }
                }
            }
            Ok(())
        });
        keep_alive.store(false, Ordering::Relaxed);
        result
    })
}

/// Checks each node and sends its index and latest result to the screen.
/// Stops when requested or when the screen's receiver is dropped.
fn poll_nodes(
    cluster: &ClusterSpec,
    client: &FleetClient,
    sender: mpsc::Sender<(usize, NodeState)>,
    keep_alive: &AtomicBool,
) {
    while keep_alive.load(Ordering::Relaxed) {
        for (index, node) in cluster.nodes.iter().enumerate() {
            if !keep_alive.load(Ordering::Relaxed) {
                return;
            }

            let state = match client.fetch_status(&node.address, &node.server_name) {
                Ok(status) => NodeState::Connected { status },
                Err(error) => NodeState::Unreachable {
                    message: error.to_string(),
                },
            };

            if sender.send((index, state)).is_err() {
                return;
            }
        }

        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Draws each node's latest state, hiding addresses on narrow terminals.
/// Long messages are clipped to the available column width.
///
/// # Panics
/// Panics if there is no state entry for a configured node.
fn draw(frame: &mut Frame, cluster: &ClusterSpec, states: &[NodeState]) {
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
    let rows = cluster.nodes.iter().enumerate().map(|(index, node)| {
        let connection = match &states[index] {
            NodeState::NotChecked => String::from("Not checked"),
            NodeState::Connected { status } => {
                if compact {
                    format!("{}/{} running", status.running, status.desired)
                } else {
                    format!("Connected: {}/{} running", status.running, status.desired)
                }
            }
            NodeState::Unreachable { message } => format!("Unreachable: {}", message),
        };
        let cells = if compact {
            vec![node.name.clone(), connection]
        } else {
            vec![node.name.clone(), node.address.clone(), connection]
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
                Constraint::Percentage(20),
                Constraint::Percentage(35),
                Constraint::Percentage(45),
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
        let states = vec![NodeState::NotChecked, NodeState::NotChecked];

        for width in [80, 40] {
            let mut terminal = Terminal::new(TestBackend::new(width, 15))
                .expect("test terminal should initialize");
            terminal
                .draw(|frame| draw(frame, &cluster, &states))
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

    #[test]
    fn draws_live_counts_and_connection_errors() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster.json"))
            .expect("example cluster should parse");
        let states = vec![
            NodeState::Connected {
                status: StatusResponse {
                    workload_name: Some(String::from("demo-service")),
                    desired: 2,
                    running: 1,
                },
            },
            NodeState::Unreachable {
                message: String::from("timeout"),
            },
        ];

        for width in [80, 40] {
            let mut terminal = Terminal::new(TestBackend::new(width, 15))
                .expect("test terminal should initialize");
            terminal
                .draw(|frame| draw(frame, &cluster, &states))
                .expect("screen should draw");
            let screen: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();

            assert!(screen.contains("1/2 running"));
            assert!(screen.contains("Unreachable"));
            assert!(!screen.contains("Not checked"));
            if width >= 60 {
                assert!(screen.contains("Connected: 1/2 running"));
                assert!(screen.contains("Unreachable: timeout"));
            }
        }
    }
}

/// The latest result of checking an agent, not the health of its application.
pub enum NodeState {
    /// No check has finished for this node yet.
    NotChecked,
    /// The agent replied with its process counts.
    Connected { status: StatusResponse },
    /// The check failed, including connection, TLS, or authentication errors.
    Unreachable { message: String },
}
