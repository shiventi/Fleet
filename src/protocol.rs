use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub workload_name: Option<String>,
    pub desired: u32,
    pub running: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    Status,
    Stop,
    Scale { replicas: u32 },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Status { status: StatusResponse },
    Error { message: String },
}

// keep the token out of debug logs
#[derive(Serialize, Deserialize)]
pub struct AuthenticatedRequest {
    pub token: String,
    pub request: Request,
}
