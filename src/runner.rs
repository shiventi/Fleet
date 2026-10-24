use crate::workload::WorkloadSpec;
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const MAX_LOG_ENTRIES: usize = 200;
const MAX_LOG_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: u64,
    pub pid: u32,
    pub stream: LogStream,
    pub text: String,
}

#[derive(Default)]
pub struct LogBuffer {
    next_id: u64,
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    fn push(&mut self, pid: u32, stream: LogStream, mut text: String) {
        if text.len() > MAX_LOG_BYTES {
            let mut end = MAX_LOG_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        while self.entries.len() >= MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }

        self.next_id += 1;

        self.entries.push_back(LogEntry { id: self.next_id, pid, stream, text });
    }
}

pub struct ProcessRunner {
    children: Vec<Child>,
    logs: Arc<Mutex<LogBuffer>>,
}

fn read_logs<R: Read>(mut reader: R, pid: u32, stream: LogStream, logs: Arc<Mutex<LogBuffer>>) -> io::Result<()> {
    let mut chunk = [0u8; 1024];
    let mut line = Vec::new();
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => {
                if !line.is_empty() {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    let mut buffer = logs.lock().unwrap_or_else(|err| err.into_inner());
                    buffer.push(pid, stream.clone(), text);
                }
                return Ok(());
            }
            Ok(count) => {
                for &byte in &chunk[..count] {
                    if byte == b'\n' {
                        let text = String::from_utf8_lossy(&line).into_owned();
                        let mut buffer = logs.lock().unwrap_or_else(|err| err.into_inner());
                        buffer.push(pid, stream.clone(), text);
                        line.clear();
                    } else if line.len() < MAX_LOG_BYTES {
                        line.push(byte);
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
}

impl ProcessRunner {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            logs: Arc::new(Mutex::new(LogBuffer::default())),
        }
    }

    pub fn start(&mut self, workload: &WorkloadSpec) -> std::io::Result<()> {
        let mut child = Command::new(&workload.program)
            .args(&workload.args)
            .process_group(0)
            // do not pass the control token to apps
            .env_remove("FLEET_TOKEN")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let pid = child.id();
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");
        self.children.push(child);

        let stdout_logs = Arc::clone(&self.logs);
        let stdout_thread = std::thread::Builder::new().spawn(move || {
            if let Err(err) = read_logs(stdout, pid, LogStream::Stdout, stdout_logs) {
                eprintln!("stdout read failed for {}: {}", pid, err);
            }
        });
        if let Err(error) = stdout_thread {
            self.stop_one();
            return Err(error);
        }

        let stderr_logs = Arc::clone(&self.logs);
        let stderr_thread = std::thread::Builder::new().spawn(move || {
            if let Err(err) = read_logs(stderr, pid, LogStream::Stderr, stderr_logs) {
                eprintln!("stderr read failed for {}: {}", pid, err);
            }
        });
        if let Err(error) = stderr_thread {
            self.stop_one();
            return Err(error);
        }

        println!("Started process: {}", pid);

        Ok(())
    }

    pub fn running_count(&mut self) -> usize {
        self.children.retain_mut(|child| {
            let status = child.try_wait().expect("failed to check process");

            match status {
                None => true,
                Some(status) => {
                    println!("Process {} exited: {}", child.id(), status);
                    let group = Pid::from_child(child);
                    match kill_process_group(group, Signal::KILL) {
                        Ok(()) => {}
                        Err(Errno::SRCH) => {}
                        Err(err) => {
                            panic!("failed to stop process: {}", err);
                        }
                    }
                    false
                }
            }
        });
        self.children.len()
    }

    pub fn stop_all(&mut self) {
        while !self.children.is_empty() {
            self.stop_one();
        }
    }

    pub fn stop_one(&mut self) {
        if let Some(mut child) = self.children.pop() {
            let group = Pid::from_child(&child);
            let mut result = kill_process_group(group, Signal::KILL);

            if result == Err(Errno::PERM) {
                let _ = child.try_wait().expect("failed to check process");
                result = kill_process_group(group, Signal::KILL);
            }

            match result {
                Ok(()) => {}
                Err(Errno::SRCH) => {}
                Err(err) => {
                    panic!("failed to stop process group: {}", err);
                }
            }
            child.wait().expect("failed to wait for process");
        }
    }

    pub fn logs_after(&self, after_id: u64) -> Vec<LogEntry> {
        let buffer = self.logs.lock().unwrap_or_else(|err| err.into_inner());
        buffer.entries.iter().filter(|entry| entry.id > after_id).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lines_and_saves_the_last_partial_line() {
        let runner = ProcessRunner::new();
        read_logs(&b"first\n\nlast"[..], 42, LogStream::Stdout, Arc::clone(&runner.logs)).unwrap();

        let entries = runner.logs_after(0);
        assert_eq!(entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>(), vec!["first", "", "last"]);
        assert_eq!(entries[0].pid, 42);
        assert!(matches!(entries[0].stream, LogStream::Stdout));
        let newer = runner.logs_after(entries[1].id);
        assert_eq!(newer.len(), 1);
        assert_eq!(newer[0].text, "last");
        assert!(runner.logs_after(entries[2].id).is_empty());
    }

    #[test]
    fn does_not_add_a_line_for_empty_output_or_after_a_final_newline() {
        let runner = ProcessRunner::new();
        read_logs(&b""[..], 42, LogStream::Stdout, Arc::clone(&runner.logs)).unwrap();
        assert!(runner.logs_after(0).is_empty());
        read_logs(&b"one\n"[..], 42, LogStream::Stdout, Arc::clone(&runner.logs)).unwrap();
        assert_eq!(runner.logs_after(0).len(), 1);
    }

    #[test]
    fn caps_long_lines_and_keeps_reading_after_them() {
        let runner = ProcessRunner::new();
        let mut output = vec![b'x'; MAX_LOG_BYTES * 3];
        output.extend_from_slice(b"\nnext\n");
        read_logs(output.as_slice(), 42, LogStream::Stderr, Arc::clone(&runner.logs)).unwrap();

        let entries = runner.logs_after(0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text.len(), MAX_LOG_BYTES);
        assert_eq!(entries[1].text, "next");
    }

    #[test]
    fn caps_log_history_and_keeps_valid_text() {
        let mut buffer = LogBuffer::default();
        for _ in 0..MAX_LOG_ENTRIES + 3 {
            buffer.push(42, LogStream::Stdout, String::from("line"));
        }
        assert_eq!(buffer.entries.len(), MAX_LOG_ENTRIES);
        assert_eq!(buffer.entries.front().unwrap().id, 4);

        let runner = ProcessRunner::new();
        let mut output = vec![b'x'; MAX_LOG_BYTES - 1];
        output.extend_from_slice(&[0xff, b'\n']);
        read_logs(output.as_slice(), 42, LogStream::Stdout, Arc::clone(&runner.logs)).unwrap();
        assert_eq!(runner.logs_after(0)[0].text.len(), MAX_LOG_BYTES - 1);
    }

    #[test]
    fn log_entries_round_trip_as_json() {
        let entry = LogEntry { id: 1, pid: 42, stream: LogStream::Stderr, text: String::from("error\n") };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: LogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.pid, 42);
        assert!(matches!(decoded.stream, LogStream::Stderr));
        assert_eq!(decoded.text, "error\n");
    }

    #[test]
    fn captures_both_pipes_from_a_real_process() {
        let workload = WorkloadSpec {
            name: String::from("logs"), replicas: 1, program: String::from("/bin/sh"),
            args: vec![String::from("-c"), String::from("printf 'out\\n'; printf 'err' >&2")],
        };
        let mut runner = ProcessRunner::new();
        runner.start(&workload).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let entries = loop {
            let entries = runner.logs_after(0);
            if entries.len() >= 2 || std::time::Instant::now() >= deadline {
                break entries;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        runner.stop_all();
        assert_eq!(runner.running_count(), 0);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| matches!(entry.stream, LogStream::Stdout) && entry.text == "out"));
        assert!(entries.iter().any(|entry| matches!(entry.stream, LogStream::Stderr) && entry.text == "err"));
    }

    #[test]
    fn drains_large_output_from_both_pipes_without_blocking_the_app() {
        let chunk = "x".repeat(1024);
        let script = format!("i=0; while [ \"$i\" -lt 256 ]; do printf '%s' '{chunk}'; printf '%s' '{chunk}' >&2; i=$((i+1)); done");
        let workload = WorkloadSpec {
            name: "large-output".into(), replicas: 1, program: "/bin/sh".into(), args: vec!["-c".into(), script],
        };
        let mut runner = ProcessRunner::new();
        runner.start(&workload).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let finished = loop {
            if runner.running_count() == 0 {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        runner.stop_all();
        assert!(finished, "large output blocked the app");
        while runner.logs_after(0).len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let entries = runner.logs_after(0);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.text.len() == MAX_LOG_BYTES));
        assert!(entries.iter().any(|entry| matches!(entry.stream, LogStream::Stdout)));
        assert!(entries.iter().any(|entry| matches!(entry.stream, LogStream::Stderr)));
    }

    #[test]
    fn stops_one_copy_and_handles_an_empty_runner() {
        let workload = WorkloadSpec {
            name: String::from("test"),
            replicas: 2,
            program: String::from("/bin/sleep"),
            args: vec![String::from("60")],
        };
        let mut runner = ProcessRunner::new();
        runner.start(&workload).expect("test process should start");
        runner.start(&workload).expect("test process should start");
        runner.stop_one();
        let remaining = runner.running_count();
        runner.stop_all();
        assert_eq!(remaining, 1);
        runner.stop_one();
        assert_eq!(runner.running_count(), 0);
    }

    #[test]
    fn stops_all_started_processes() {
        let workload = WorkloadSpec {
            name: String::from("test"),
            replicas: 1,
            program: String::from("/bin/sleep"),
            args: vec![String::from("60")],
        };

        let mut runner = ProcessRunner::new();
        runner.start(&workload).expect("test process should start");

        assert_eq!(runner.running_count(), 1);

        runner.stop_all();

        assert_eq!(runner.running_count(), 0);
    }

    #[test]
    fn failed_start_does_not_add_a_process() {
        let workload = WorkloadSpec {
            name: String::from("missing-program"),
            replicas: 1,
            program: String::from("/fleet-test-missing-directory/missing-program"),
            args: Vec::new(),
        };
        let mut runner = ProcessRunner::new();

        let result = runner.start(&workload);

        assert!(result.is_err());
        assert_eq!(runner.running_count(), 0);
    }
}
