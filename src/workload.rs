#[derive(Debug)]
pub struct WorkloadSpec {
    pub name: String,
    pub replicas: u32,
    pub program: String,
    pub args: Vec<String>
}
