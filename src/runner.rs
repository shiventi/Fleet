use crate::workload::WorkloadSpec;
use std::process::Child;
use std::process::Command;

pub struct ProcessRunner {
    children: Vec<Child>,
}

impl ProcessRunner {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn start(&mut self, workload: &WorkloadSpec) -> std::io::Result<()> {
        let child = Command::new(&workload.program)
            .args(&workload.args)
            // do not pass the control token to apps
            .env_remove("FLEET_TOKEN")
            .spawn()?;

        println!("Started process: {}", child.id());

        self.children.push(child);

        Ok(())
    }

    pub fn running_count(&mut self) -> usize {
        self.children.retain_mut(|child| {
            let status = child.try_wait().expect("failed to check process");

            match status {
                None => true,
                Some(_) => false,
            }
        });
        self.children.len()
    }

    pub fn stop_all(&mut self) {
        //todo: stop child trees
        for child in &mut self.children {
            let status = child.try_wait().expect("failed to check process");

            match status {
                None => {
                    let _ = child.kill().expect("failed to stop process");
                }
                Some(_) => {}
            }

            let _ = child.wait().expect("failed to wait for process");
        }
        self.children.clear();
    }

    pub fn stop_one(&mut self) {
        if let Some(mut child) = self.children.pop() {
            child.kill().expect("failed to stop process");
            child.wait().expect("failed to wait for process");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
