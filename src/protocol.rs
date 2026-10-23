use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub workload_name: Option<String>,
    pub desired: u32,
    pub running: usize,
    #[serde(default)]
    pub metrics: Option<HostMetrics>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    Status,
    Stop,
    Scale { replicas: u32 },
    StatusApp { workload_name: String },
    StopApp { workload_name: String },
    ScaleApp { workload_name: String, replicas: u32 },
    ListApps,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Status { status: StatusResponse },
    Error { message: String },
    Apps { apps: Vec<StatusResponse> },
}

// keep the token out of debug logs
#[derive(Serialize, Deserialize)]
pub struct AuthenticatedRequest {
    pub token: String,
    pub request: Request,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HostMetrics {
    pub cpu_percent: Option<f32>,
    pub used_mem: u64,
    pub total_mem: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_old_status_replies_without_metrics() {
        let status: StatusResponse = serde_json::from_str(r#"{"workload_name":"demo","desired":1,"running":1}"#).unwrap();
        assert!(status.metrics.is_none());
        assert!(serde_json::from_str::<StatusResponse>(r#"{"workload_name":"demo"}"#).is_err());
    }

    #[test]
    fn keeps_host_readings_in_status_replies() {
        let status = StatusResponse {
            workload_name: Some(String::from("demo")), desired: 1, running: 1,
            metrics: Some(HostMetrics { cpu_percent: Some(25.0), used_mem: 1024, total_mem: 4096 }),
        };
        let decoded: StatusResponse = serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
        let metrics = decoded.metrics.unwrap();
        assert_eq!(metrics.cpu_percent, Some(25.0));
        assert_eq!(metrics.used_mem, 1024);
        assert_eq!(metrics.total_mem, 4096);
    }
}
