use crate::runner::LogEntry;
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
pub struct BundleInfo {
    pub size: u64,
    pub sha256: String,
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
    LogsApp { workload_name: String, after_id: u64 },
    Upload { bundle: BundleInfo },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Status { status: StatusResponse },
    Error { message: String },
    Apps { apps: Vec<StatusResponse> },
    Logs { entries: Vec<LogEntry> },
    UploadReady,
    Uploaded { sha256: String },
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
    fn bundle_info_keeps_size_and_hash_when_encoded_as_json() {
        let info = BundleInfo { size: 3, sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into() };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["size"], 3);
        assert_eq!(json["sha256"], info.sha256);
        let decoded: BundleInfo = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.size, info.size);
        assert_eq!(decoded.sha256, info.sha256);
        assert!(serde_json::from_str::<BundleInfo>(r#"{"size":-1,"sha256":"abc"}"#).is_err());
        assert!(serde_json::from_str::<BundleInfo>(r#"{"size":3}"#).is_err());
    }

    #[test]
    fn upload_messages_keep_their_fields_when_encoded_as_json() {
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let json = serde_json::json!({ "command": "upload", "bundle": { "size": 3, "sha256": hash } });
        let request: Request = serde_json::from_value(json.clone()).unwrap();
        assert!(matches!(&request, Request::Upload { bundle } if bundle.size == 3 && bundle.sha256 == hash));
        assert_eq!(serde_json::to_value(request).unwrap(), json);

        for (reply, expected) in [
            (Response::UploadReady, serde_json::json!("UploadReady")),
            (Response::Uploaded { sha256: hash.into() }, serde_json::json!({ "Uploaded": { "sha256": hash } })),
        ] {
            let json = serde_json::to_value(reply).unwrap();
            assert_eq!(json, expected);
            let decoded: Response = serde_json::from_value(json).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
        }
    }

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
