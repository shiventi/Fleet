use super::*;
use crate::client::log_text;
use crate::runner::{LogEntry, LogStream};
use std::collections::VecDeque;

pub(super) struct LogView {
    pub index: usize,
    name: String,
    generation: u64,
    after_id: u64,
    entries: VecDeque<LogEntry>,
    pending: bool,
    next_check: Instant,
    error: Option<String>,
    checked: bool,
    offset: usize,
}

pub(super) struct LogCommand {
    index: usize,
    name: String,
    generation: u64,
    after_id: u64,
}

pub(super) struct LogResult {
    generation: u64,
    reply: Result<Vec<LogEntry>, String>,
}

impl LogView {
    pub fn new(index: usize, name: String, generation: u64) -> Self {
        Self {
            index, name, generation, after_id: 0, entries: VecDeque::new(),
            pending: false, next_check: Instant::now(), error: None, checked: false, offset: 0,
        }
    }

    pub fn request(&mut self, sender: &mpsc::SyncSender<LogCommand>) {
        if self.pending || Instant::now() < self.next_check {
            return;
        }
        let command = LogCommand { index: self.index, name: self.name.clone(), generation: self.generation, after_id: self.after_id };
        match sender.try_send(command) {
            Ok(()) => self.pending = true,
            Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.error = Some(String::from("Log worker stopped"));
                self.next_check = Instant::now() + Duration::from_secs(2);
            }
        }
    }

    pub fn apply(&mut self, result: LogResult) {
        // a reply for an old view must not change this one
        if result.generation != self.generation {
            return;
        }
        self.pending = false;
        self.checked = true;
        self.next_check = Instant::now() + Duration::from_secs(2);
        let entries = match result.reply {
            Ok(entries) => entries,
            Err(message) => {
                self.error = Some(message);
                return;
            }
        };
        let mut previous = self.after_id;
        for entry in &entries {
            if entry.id <= previous {
                self.error = Some(String::from("Log IDs must increase"));
                return;
            }
            previous = entry.id;
        }
        self.error = None;
        self.next_check = Instant::now() + if entries.is_empty() { Duration::from_millis(500) } else { Duration::ZERO };
        if self.offset > 0 {
            self.offset = self.offset.saturating_add(entries.len());
        }
        for entry in entries {
            self.after_id = entry.id;
            self.entries.push_back(entry);
            if self.entries.len() > 200 {
                self.entries.pop_front();
            }
        }
        self.offset = self.offset.min(self.entries.len().saturating_sub(1));
    }

    pub fn scroll(&mut self, key: KeyCode) {
        self.offset = match key {
            KeyCode::Up => self.offset.saturating_add(1),
            KeyCode::PageUp => self.offset.saturating_add(10),
            KeyCode::Down => self.offset.saturating_sub(1),
            KeyCode::PageDown => self.offset.saturating_sub(10),
            KeyCode::Home => self.entries.len(),
            KeyCode::End => 0,
            _ => self.offset,
        }.min(self.entries.len().saturating_sub(1));
    }

    pub fn draw(&self, frame: &mut Frame, node: &str) {
        if frame.area().width < 32 || frame.area().height < 8 {
            frame.render_widget(Paragraph::new("Terminal too small"), frame.area());
            return;
        }
        let areas = Layout::vertical([Constraint::Length(2), Constraint::Min(0), Constraint::Length(2)]).margin(1).split(frame.area());
        frame.render_widget(Paragraph::new(format!("Fleet / {} / {}", log_text(node), log_text(&self.name))).style(Style::default().fg(ACCENT)), areas[0]);
        let block = Block::default().title(" Logs ").borders(Borders::ALL).border_style(Style::default().fg(ACCENT)).padding(Padding::horizontal(1));
        let height = usize::from(block.inner(areas[1]).height);
        let start = self.entries.len().saturating_sub(height).saturating_sub(self.offset);
        let lines: Vec<Line> = self.entries.iter().skip(start).take(height).map(|entry| {
            let (stream, color) = match entry.stream {
                LogStream::Stdout => ("stdout", Color::Gray),
                LogStream::Stderr => ("stderr", Color::Red),
            };
            Line::from(vec![
                Span::styled(format!("{} {} {}  ", entry.id, entry.pid, stream), Style::default().fg(Color::DarkGray)),
                Span::styled(log_text(&entry.text), Style::default().fg(color)),
            ])
        }).collect();
        let content = if lines.is_empty() {
            Paragraph::new(if self.checked { "No log lines yet" } else { "Loading logs..." }).style(Style::default().fg(Color::Gray))
        } else {
            Paragraph::new(lines)
        };
        frame.render_widget(content.block(block), areas[1]);
        let (status, color) = match &self.error {
            Some(error) => (format!("{}; retrying", log_text(error)), Color::Red),
            None => (format!("{} | {} lines", if self.offset == 0 { "Following" } else { "Scrolled" }, self.entries.len()), Color::Gray),
        };
        frame.render_widget(Paragraph::new(vec![
            Line::styled(status, Style::default().fg(color)),
            Line::styled("Esc: apps  Up/Down: scroll  End: follow", Style::default().fg(Color::DarkGray)),
        ]), areas[2]);
    }
}

pub(super) fn worker(cluster: &ClusterSpec, clients: &[FleetClient], receiver: mpsc::Receiver<LogCommand>, sender: mpsc::Sender<LogResult>, keep_alive: &AtomicBool) {
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
            Some((node, client)) => client.fetch_logs(&node.address, &command.name, command.after_id, &node.server_name).map_err(|error| error.to_string()),
            None => Err(String::from("Unknown node")),
        };
        if sender.send(LogResult { generation: command.generation, reply }).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn result(generation: u64, ids: std::ops::RangeInclusive<u64>) -> LogResult {
        LogResult { generation, reply: Ok(ids.map(|id| LogEntry {
            id, pid: 42, stream: LogStream::Stdout, text: format!("line {id}"),
        }).collect()) }
    }

    #[test]
    fn queues_only_one_request_and_keeps_the_app_and_cursor() {
        let mut view = LogView::new(1, "worker".into(), 3);
        let (sender, receiver) = mpsc::sync_channel(1);
        view.request(&sender);
        let command = receiver.try_recv().unwrap();
        assert_eq!(command.index, 1);
        assert_eq!(command.name, "worker");
        assert_eq!(command.after_id, 0);
        assert_eq!(command.generation, 3);
        view.request(&sender);
        assert!(receiver.try_recv().is_err());
        view.apply(result(3, 1..=2));
        view.request(&sender);
        assert_eq!(receiver.try_recv().unwrap().after_id, 2);
    }

    #[test]
    fn ignores_replies_from_a_closed_or_different_app_view() {
        let mut view = LogView::new(1, "worker".into(), 3);
        view.pending = true;
        view.apply(result(2, 1..=2));
        assert!(view.entries.is_empty());
        assert!(view.pending);
        assert_eq!(view.after_id, 0);
        view.apply(result(3, 1..=2));
        assert_eq!(view.entries.len(), 2);
        assert!(!view.pending);
    }

    #[test]
    fn keeps_a_bounded_history_and_holds_the_scrolled_position() {
        let mut view = LogView::new(0, "api".into(), 1);
        view.apply(result(1, 1..=30));
        view.scroll(KeyCode::PageUp);
        assert_eq!(view.offset, 10);
        view.apply(result(1, 31..=35));
        assert_eq!(view.offset, 15);
        view.scroll(KeyCode::End);
        assert_eq!(view.offset, 0);
        view.apply(result(1, 36..=250));
        assert_eq!(view.entries.len(), 200);
        assert_eq!(view.entries.front().unwrap().id, 51);
        assert_eq!(view.after_id, 250);
        view.scroll(KeyCode::Home);
        assert_eq!(view.offset, 199);
        view.scroll(KeyCode::Down);
        assert_eq!(view.offset, 198);
    }

    #[test]
    fn shows_errors_without_losing_logs_or_advancing_the_cursor() {
        let mut view = LogView::new(0, "api".into(), 1);
        view.apply(result(1, 1..=2));
        view.apply(LogResult { generation: 1, reply: Err("offline".into()) });
        assert_eq!(view.error.as_deref(), Some("offline"));
        assert_eq!(view.entries.len(), 2);
        assert_eq!(view.after_id, 2);
        assert!(view.next_check > Instant::now());
        view.apply(result(1, 2..=3));
        assert_eq!(view.after_id, 2);
        assert_eq!(view.entries.len(), 2);
        assert_eq!(view.error.as_deref(), Some("Log IDs must increase"));
        view.apply(result(1, 3..=3));
        assert!(view.error.is_none());
        assert_eq!(view.after_id, 3);
    }

    #[test]
    fn draws_safe_log_text_and_stderr_in_wide_and_narrow_views() {
        let mut view = LogView::new(0, "api".into(), 1);
        view.apply(LogResult { generation: 1, reply: Ok(vec![LogEntry {
            id: 1, pid: 42, stream: LogStream::Stderr, text: "warning\x1b[2J".into(),
        }]) });
        for (width, height) in [(40, 18), (120, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| view.draw(frame, "node-one")).unwrap();
            let buffer = terminal.backend().buffer();
            let screen: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(screen.contains("Fleet / node-one / api"));
            assert!(screen.contains("Logs"));
            assert!(screen.contains("Following"));
            assert!(!screen.contains('\x1b'));
            let position = buffer.content.windows(7).position(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>() == "warning").unwrap();
            assert!(buffer.content[position..position + 7].iter().all(|cell| cell.fg == Color::Red));
            if width == 120 {
                assert!(screen.contains("\\u{1b}[2J"));
            }
        }
    }

    #[test]
    fn draws_loading_empty_errors_and_small_terminals() {
        let mut view = LogView::new(0, "api".into(), 1);
        let mut terminal = Terminal::new(TestBackend::new(80, 18)).unwrap();
        terminal.draw(|frame| view.draw(frame, "node")).unwrap();
        let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("Loading logs"));
        view.apply(LogResult { generation: 1, reply: Ok(Vec::new()) });
        terminal.draw(|frame| view.draw(frame, "node")).unwrap();
        let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("No log lines yet"));
        view.apply(LogResult { generation: 1, reply: Err("offline".into()) });
        terminal.draw(|frame| view.draw(frame, "node")).unwrap();
        let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("offline; retrying"));
        for (width, height) in [(24, 18), (80, 6)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| view.draw(frame, "node")).unwrap();
            let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
            assert!(screen.contains("Terminal too small"));
        }
    }
}
