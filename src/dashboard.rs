use crate::client::FleetClient;
use crate::controller::ClusterSpec;
use crate::protocol::StatusResponse;
use crate::tls::TlsFiles;
use crate::workload::MAX_REPLICAS;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Block, Borders, Padding, Paragraph, Row, Table, TableState, Wrap};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const ACCENT: Color = Color::Rgb(50, 108, 229);

pub fn run(cluster: &ClusterSpec, files: &TlsFiles) -> io::Result<()> {
    let client = FleetClient::new(files)?;
    let clients = vec![client; cluster.nodes.len()];
    run_clients(cluster, &clients)
}

pub fn run_device(cluster: &ClusterSpec) -> io::Result<()> {
    let clients = cluster.device_clients()?;
    run_clients(cluster, &clients)
}

fn run_clients(cluster: &ClusterSpec, clients: &[FleetClient]) -> io::Result<()> {
    cluster.validate().map_err(io::Error::other)?;
    if cluster.nodes.len() != clients.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Each node needs one client"));
    }

    let mut states: Vec<NodeState> = Vec::new();
    for _ in &cluster.nodes {
        states.push(NodeState::NotChecked);
    }

    let (sender, receiver) = mpsc::channel::<PollResult>();
    let (command_sender, command_receiver) = mpsc::sync_channel::<ScaleCommand>(1);
    let (result_sender, result_receiver) = mpsc::channel::<ScaleResult>();
    let mut pending: Option<usize> = None;
    let mut notice = String::new();
    let mut updated_after = vec![Instant::now(); cluster.nodes.len()];
    let keep_alive = AtomicBool::new(true);

    let mut selection = TableState::default();
    selection.select(Some(0));
    std::thread::scope(|scope| {
        let _shutdown = ShutdownFlag(&keep_alive);
        scope.spawn(|| poll_nodes(cluster, clients, sender, &keep_alive));
        scope.spawn(|| {
            scale_worker(
                cluster,
                clients,
                command_receiver,
                result_sender,
                &keep_alive,
            )
        });
        let result = ratatui::run(|terminal| {
            loop {
                for update in receiver.try_iter() {
                    apply_poll(update, &mut states, pending, &updated_after);
                }
                for result in result_receiver.try_iter() {
                    pending = None;
                    updated_after[result.index] = result.finished_at;
                    match result.reply {
                        Ok(status) => {
                            notice = format!(
                                "{}: target set to {}",
                                cluster.nodes[result.index].name, status.desired
                            );
                            states[result.index] = NodeState::Connected { status };
                        }
                        Err(message) => {
                            // a lost reply may hide a change
                            states[result.index] = NodeState::NotChecked;
                            notice = format!(
                                "{}: scale failed: {}. Check status before retrying.",
                                cluster.nodes[result.index].name, message
                            );
                        }
                    }
                }
                terminal.draw(|frame| draw(frame, cluster, &states, &mut selection, &notice))?;

                if event::poll(Duration::from_millis(100))? {
                    if let Event::Key(key) = event::read()? {
                        let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL));

                        if key.kind == KeyEventKind::Press {
                            if quit {
                                break;
                            }

                            let index = selection.selected().unwrap_or(0);
                            let code = scale_key(key);

                            match code {
                                KeyCode::Down => {
                                    let new_indx = (index + 1).min(cluster.nodes.len() - 1);
                                    selection.select(Some(new_indx))
                                }
                                KeyCode::Up => {
                                    let new_indx = index.saturating_sub(1);
                                    selection.select(Some(new_indx))
                                }
                                KeyCode::Char('=') | KeyCode::Char('+') | KeyCode::Char('-') => {
                                    if let Some(index) = pending {
                                        notice = format!(
                                            "{}: waiting for scale reply",
                                            cluster.nodes[index].name
                                        );
                                        continue;
                                    }
                                    match scale_command(index, code, &states) {
                                        Ok(command) => {
                                            let replicas = command.replicas;
                                            match command_sender.try_send(command) {
                                                Ok(()) => {
                                                    pending = Some(index);
                                                    notice = format!(
                                                        "{}: setting target to {}...",
                                                        cluster.nodes[index].name, replicas
                                                    );
                                                }
                                                Err(error) => {
                                                    notice = format!(
                                                        "Could not send scale command: {}",
                                                        error
                                                    )
                                                }
                                            }
                                        }
                                        Err(message) => notice = message,
                                    }
                                }
                                _ => {}
                            }
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

fn poll_nodes(cluster: &ClusterSpec, clients: &[FleetClient], sender: mpsc::Sender<PollResult>, keep_alive: &AtomicBool) {
    while keep_alive.load(Ordering::Relaxed) {
        for (index, (node, client)) in cluster.nodes.iter().zip(clients.iter()).enumerate() {
            if !keep_alive.load(Ordering::Relaxed) {
                return;
            }

            let started_at = Instant::now();
            let state = match client.fetch_status(&node.address, &node.server_name) {
                Ok(status) => NodeState::Connected { status },
                Err(error) => NodeState::Unreachable {
                    message: error.to_string(),
                },
            };

            if sender
                .send(PollResult {
                    index,
                    state,
                    started_at,
                })
                .is_err()
            {
                return;
            }
        }

        for _ in 0..20 {
            if !keep_alive.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn scale_worker(cluster: &ClusterSpec, clients: &[FleetClient], receiver: mpsc::Receiver<ScaleCommand>, sender: mpsc::Sender<ScaleResult>, keep_alive: &AtomicBool) {
    while keep_alive.load(Ordering::Relaxed) {
        let command = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };

        if !keep_alive.load(Ordering::Relaxed) {
            return;
        }

        let reply = match cluster.nodes.get(command.index).zip(clients.get(command.index)) {
            Some((node, client)) => client
                .scale(&node.address, command.replicas, &node.server_name)
                .map_err(|error| error.to_string()),
            None => Err(String::from("Unknown node")),
        };
        let result = ScaleResult {
            index: command.index,
            reply,
            finished_at: Instant::now(),
        };

        if sender.send(result).is_err() {
            return;
        }
    }
}

fn apply_poll(update: PollResult, states: &mut [NodeState], pending: Option<usize>, updated_after: &[Instant]) {
    // ignore stale poll replies.
    if pending != Some(update.index) && update.started_at >= updated_after[update.index] {
        states[update.index] = update.state;
    }
}

fn scale_key(key: KeyEvent) -> KeyCode {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('7') | KeyCode::Char('_')) {
        KeyCode::Char('-')
    } else {
        key.code
    }
}

fn scale_command(index: usize, key: KeyCode, states: &[NodeState]) -> Result<ScaleCommand, String> {
    let Some(NodeState::Connected { status }) = states.get(index) else {
        return Err(String::from(
            "Wait for a successful node check before scaling.",
        ));
    };
    let replicas = match key {
        KeyCode::Char('=') | KeyCode::Char('+') => status.desired.saturating_add(1).min(MAX_REPLICAS),
        KeyCode::Char('-') => status.desired.saturating_sub(1),
        _ => return Err(String::from("Not a scale key")),
    };
    if replicas == status.desired {
        return Err(format!(
            "Copy count must stay between 0 and {}.",
            MAX_REPLICAS
        ));
    }
    Ok(ScaleCommand { index, replicas })
}

fn draw_meter(frame: &mut Frame, area: Rect, label: &str, reading: String, ratio: f64, color: Color) {
    let parts = Layout::horizontal([Constraint::Length(2), Constraint::Min(0)]).spacing(1).split(area);
    let bars = [Bar::default().value((ratio * 1000.0).round() as u64).text_value(String::new())];
    let chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .max(1000)
        .bar_width(2)
        .style(Style::default().bg(Color::Rgb(38, 42, 47)))
        .bar_style(Style::default().fg(color).bg(Color::Rgb(38, 42, 47)));
    frame.render_widget(chart, parts[0]);
    let lines = vec![Line::from(label).style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)), Line::from(reading)];
    frame.render_widget(Paragraph::new(lines), parts[1]);
}

fn draw(frame: &mut Frame, cluster: &ClusterSpec, states: &[NodeState], selection: &mut TableState, notice: &str) {
    let compact = frame.area().width.saturating_sub(2) < 60;
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(2),
        Constraint::Length(if compact { 4 } else { 3 }),
    ])
    .margin(1)
    .split(frame.area());
    let heading = Layout::horizontal([Constraint::Min(0), Constraint::Length(12)]).split(areas[0]);
    let title = Paragraph::new("Fleet").style(Style::default().fg(ACCENT));
    let count = Paragraph::new(format!("Nodes: {}", cluster.nodes.len()))
        .alignment(Alignment::Right)
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(title, heading[0]);
    frame.render_widget(count, heading[1]);

    let index = selection.selected().unwrap_or(0);
    let metrics = match states.get(index) {
        Some(NodeState::Connected { status }) => status.metrics.as_ref(),
        _ => None,
    };

    let node_name = cluster.nodes.get(index).map(|node| node.name.as_str()).unwrap_or("No node selected");
    let footer = if compact {
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(areas[3])
    } else {
        Layout::horizontal([Constraint::Min(0), Constraint::Length(42)]).spacing(3).split(areas[3])
    };
    let mut host = vec![Line::from(vec![
        Span::styled("Host: ", Style::default().fg(Color::Gray)),
        Span::styled(node_name, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
    ])];
    if !compact {
        if let Some(node) = cluster.nodes.get(index) {
            host.push(Line::from(node.address.as_str()).style(Style::default().fg(Color::DarkGray)));
        }
    }
    frame.render_widget(Paragraph::new(host), footer[0]);
    let inner = footer[1];
    let panels = Layout::horizontal([Constraint::Length(14), Constraint::Min(0)]).spacing(2).split(inner);

    if let Some(metrics) = metrics {
        let (ratio, label) = match metrics.cpu_percent {
            Some(cpu) if cpu.is_finite() => (
                (f64::from(cpu) / 100.0).clamp(0.0, 1.0),
                format!("{:.1}%", cpu),
            ),
            _ => (0.0, String::from("Warming up")),
        };

        draw_meter(frame, panels[0], "CPU:", label, ratio, ACCENT);

        let (ratio, label) = if metrics.total_mem > 0 {
            let used_gib = metrics.used_mem as f64 / 1_073_741_824.0;
            let total_gib = metrics.total_mem as f64 / 1_073_741_824.0;
            (
                (metrics.used_mem as f64 / metrics.total_mem as f64).clamp(0.0, 1.0),
                format!("{:.1} / {:.1} GiB", used_gib, total_gib),
            )
        } else {
            (0.0, String::from("Unavailable"))
        };
        draw_meter(frame, panels[1], "RAM:", label, ratio, Color::Cyan);
    } else {
        frame.render_widget(Paragraph::new("Metrics unavailable").style(Style::default().fg(Color::Gray)), inner);
    }

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
        .block(block)
        .row_highlight_style(Style::default().bg(ACCENT).fg(Color::White));
    frame.render_stateful_widget(table, areas[1], selection);
    frame.render_widget(
        Paragraph::new(notice)
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: true }),
        areas[2],
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn connected(desired: u32, running: usize) -> NodeState {
        NodeState::Connected {
            status: StatusResponse {
                workload_name: Some(String::from("demo-service")),
                desired,
                running,
                metrics: None,
            },
        }
    }

    #[test]
    fn rejects_missing_clients_before_opening_the_terminal() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster-device.json")).unwrap();
        let error = run_clients(&cluster, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "Each node needs one client");
    }

    #[test]
    fn rejects_empty_clusters_before_opening_the_terminal() {
        let cluster = ClusterSpec { nodes: Vec::new() };
        assert!(run_clients(&cluster, &[]).is_err());
    }

    #[test]
    fn scale_keys_use_the_selected_nodes_target() {
        let states = vec![connected(1, 1), connected(3, 1)];
        let command = scale_command(1, KeyCode::Char('+'), &states).expect("scale up");
        assert_eq!(command.index, 1);
        assert_eq!(command.replicas, 4);
        let command = scale_command(1, KeyCode::Char('='), &states).expect("scale up without Shift");
        assert_eq!(command.index, 1);
        assert_eq!(command.replicas, 4);
        let command = scale_command(1, KeyCode::Char('-'), &states).expect("scale down");
        assert_eq!(command.replicas, 2);
    }

    #[test]
    fn control_minus_accepts_explicit_and_legacy_key_events() {
        let states = vec![connected(2, 2)];
        for character in ['-', '7', '_'] {
            let code = scale_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL));
            let command = scale_command(0, code, &states).expect("Ctrl-minus should scale down");
            assert_eq!(command.replicas, 1);
        }
        for character in ['7', '_'] {
            let code = scale_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
            assert!(scale_command(0, code, &states).is_err());
        }
        let code = scale_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::CONTROL));
        assert!(scale_command(0, code, &[connected(0, 0)]).is_err());
    }

    #[test]
    fn scale_keys_enforce_limits_and_allow_restarting_from_zero() {
        let states = vec![connected(0, 0), connected(MAX_REPLICAS, 1)];
        assert!(scale_command(0, KeyCode::Char('-'), &states).is_err());
        assert!(scale_command(1, KeyCode::Char('+'), &states).is_err());
        assert!(scale_command(1, KeyCode::Char('='), &states).is_err());
        assert_eq!(
            scale_command(0, KeyCode::Char('+'), &states)
                .expect("restart")
                .replicas,
            1
        );
    }

    #[test]
    fn scale_keys_reject_unchecked_or_unreachable_nodes() {
        let states = vec![
            NodeState::NotChecked,
            NodeState::Unreachable {
                message: String::from("timeout"),
            },
        ];
        for index in 0..3 {
            assert!(scale_command(index, KeyCode::Char('+'), &states).is_err());
        }
    }

    #[test]
    fn old_polls_cannot_undo_a_scale_reply() {
        let before = Instant::now();
        let after = before + Duration::from_secs(1);
        let mut states = vec![connected(2, 1)];
        apply_poll(
            PollResult {
                index: 0,
                state: connected(1, 1),
                started_at: before,
            },
            &mut states,
            None,
            &[after],
        );
        let NodeState::Connected { status } = &states[0] else {
            panic!("connected state expected");
        };
        assert_eq!(status.desired, 2);

        apply_poll(
            PollResult {
                index: 0,
                state: connected(2, 2),
                started_at: after,
            },
            &mut states,
            None,
            &[after],
        );
        let NodeState::Connected { status } = &states[0] else {
            panic!("connected state expected");
        };
        assert_eq!(status.running, 2);
    }

    #[test]
    fn pending_scale_ignores_only_that_nodes_poll() {
        let now = Instant::now();
        let mut states = vec![connected(1, 1), NodeState::NotChecked];
        let times = vec![now; 2];
        apply_poll(
            PollResult {
                index: 0,
                state: connected(5, 5),
                started_at: now,
            },
            &mut states,
            Some(0),
            &times,
        );
        let NodeState::Connected { status } = &states[0] else {
            panic!("connected state expected");
        };
        assert_eq!(status.desired, 1);
        apply_poll(
            PollResult {
                index: 1,
                state: connected(2, 2),
                started_at: now,
            },
            &mut states,
            Some(0),
            &times,
        );
        assert!(matches!(&states[1], NodeState::Connected { .. }));
    }

    #[test]
    fn shutdown_guard_stops_workers_on_unwind() {
        let keep_alive = AtomicBool::new(true);
        let result = std::panic::catch_unwind(|| {
            let _shutdown = ShutdownFlag(&keep_alive);
            panic!("terminal failed");
        });
        assert!(result.is_err());
        assert!(!keep_alive.load(Ordering::Relaxed));
    }

    #[test]
    fn draws_metrics_for_the_selected_node() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster.json")).unwrap();
        let mut states = vec![connected(1, 1), connected(2, 2)];
        if let NodeState::Connected { status } = &mut states[1] {
            status.metrics = Some(crate::protocol::HostMetrics {
                cpu_percent: Some(25.0),
                used_mem: 2 * 1_073_741_824,
                total_mem: 8 * 1_073_741_824,
            });
        }
        for width in [80, 40, 160] {
            let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
            let mut selection = TableState::default();
            for (index, expected) in [(1, "2.0 / 8.0 GiB"), (0, "Metrics unavailable")] {
                selection.select(Some(index));
                terminal.draw(|frame| draw(frame, &cluster, &states, &mut selection, "")).unwrap();
                let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
                assert!(screen.contains(expected));
                if index == 1 {
                    assert!(screen.contains("25.0%"));
                    assert!(screen.contains("CPU:"));
                    assert!(screen.contains("RAM:"));
                    assert!(!screen.contains('|'));
                    for label in ["CPU:", "RAM:"] {
                        let buffer = terminal.backend().buffer();
                        let position = buffer.content.windows(label.len()).position(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>() == label).unwrap();
                        assert!(buffer.content[position..position + label.len()].iter().all(|cell| cell.modifier.contains(Modifier::BOLD)));
                        assert!(position % usize::from(width) >= usize::from(width.saturating_sub(43)));
                        assert_eq!(position / usize::from(width), 14);
                    }
                }
            }
        }
    }

    #[test]
    fn meters_fill_up_from_the_bottom() {
        for (ratio, filled_rows) in [(0.0, 0), (0.25, 1), (0.75, 3), (1.0, 4)] {
            let mut terminal = Terminal::new(TestBackend::new(30, 4)).unwrap();
            terminal.draw(|frame| draw_meter(frame, frame.area(), "CPU:", String::from("test reading"), ratio, ACCENT)).unwrap();
            let buffer = terminal.backend().buffer();
            for y in 0..4 {
                assert_eq!(buffer[(0, y)].symbol() != " ", y >= 4 - filled_rows);
            }
            let label: String = (3..7).map(|x| buffer[(x, 0)].symbol()).collect();
            assert_eq!(label, "CPU:");
        }
    }

    #[test]
    fn keeps_the_table_full_height_with_metrics_below() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster.json")).unwrap();
        let states = vec![NodeState::NotChecked, NodeState::NotChecked];
        for (width, height) in [(40, 18), (80, 18), (160, 40)] {
            let mut selection = TableState::default();
            selection.select(Some(0));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &cluster, &states, &mut selection, "")).unwrap();
            let buffer = terminal.backend().buffer();
            let metrics_height = if width < 62 { 4 } else { 3 };
            let metrics_y = height - metrics_height - if width < 62 { 0 } else { 1 };
            let row: String = (0..width).map(|x| buffer[(x, metrics_y)].symbol()).collect();
            assert!(row.contains("Metrics unavailable"));
            assert_ne!(buffer[(1, height - metrics_height - 4)].symbol(), " ");
            let row: String = (0..width).map(|x| buffer[(x, height - metrics_height - 1)].symbol()).collect();
            assert!(row.starts_with(" Host: local-one"));
            let row: String = (0..width).map(|x| buffer[(x, 8)].symbol()).collect();
            assert!(row.contains("local-two"));
        }
    }

    #[test]
    fn handles_missing_and_full_memory_readings() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster.json")).unwrap();
        let mut selection = TableState::default();
        selection.select(Some(0));
        let mut terminal = Terminal::new(TestBackend::new(80, 18)).unwrap();
        for total_mem in [0, 1_073_741_824] {
            let mut states = vec![connected(1, 1), connected(1, 1)];
            if let NodeState::Connected { status } = &mut states[0] {
                status.metrics = Some(crate::protocol::HostMetrics {
                    cpu_percent: None,
                    used_mem: 2 * 1_073_741_824,
                    total_mem,
                });
            }
            terminal.draw(|frame| draw(frame, &cluster, &states, &mut selection, "")).unwrap();
            let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
            assert!(screen.contains("Warming up"));
            assert!(screen.contains(if total_mem == 0 { "Unavailable" } else { "2.0 / 1.0 GiB" }));
        }
    }

    #[test]
    fn draws_nodes_in_normal_and_compact_terminals() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster.json"))
            .expect("example cluster should parse");
        let states = vec![NodeState::NotChecked, NodeState::NotChecked];

        let mut selection = TableState::default();
        selection.select(Some(0));

        for width in [80, 40] {
            let mut terminal = Terminal::new(TestBackend::new(width, 18))
                .expect("test terminal should initialize");
            terminal
                .draw(|frame| draw(frame, &cluster, &states, &mut selection, ""))
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
                    metrics: None,
                },
            },
            NodeState::Unreachable {
                message: String::from("timeout"),
            },
        ];

        for width in [80, 40] {
            let mut selection = TableState::default();
            selection.select(Some(0));
            let mut terminal = Terminal::new(TestBackend::new(width, 18))
                .expect("test terminal should initialize");
            terminal
                .draw(|frame| {
                    draw(
                        frame,
                        &cluster,
                        &states,
                        &mut selection,
                        "local-one: target set to 2",
                    )
                })
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
            assert!(screen.contains("local-one: target set to 2"));
            if width >= 60 {
                assert!(screen.contains("Connected: 1/2 running"));
                assert!(screen.contains("Unreachable: timeout"));
            }
        }
    }
}

pub enum NodeState {
    NotChecked,
    Connected { status: StatusResponse },
    Unreachable { message: String },
}

struct ScaleCommand {
    index: usize,
    replicas: u32,
}

struct ScaleResult {
    index: usize,
    reply: Result<StatusResponse, String>,
    finished_at: Instant,
}

struct PollResult {
    index: usize,
    state: NodeState,
    started_at: Instant,
}

struct ShutdownFlag<'a>(&'a AtomicBool);

impl Drop for ShutdownFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}
