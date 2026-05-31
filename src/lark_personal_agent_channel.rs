pub const REGISTRATION_SOURCE: &str = "agents-router";
pub const SDK_SOURCE_QUERY_VALUE: &str = "node-sdk/agents-router";
pub const USER_AGENT: &str = "oapi-node-sdk/1.66.0 source/agents-router channel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationSourceStatus {
    Current,
    Missing,
    Mismatch,
}

pub fn registration_source_status(source: Option<&str>) -> RegistrationSourceStatus {
    match source {
        Some(REGISTRATION_SOURCE) => RegistrationSourceStatus::Current,
        Some(_) => RegistrationSourceStatus::Mismatch,
        None => RegistrationSourceStatus::Missing,
    }
}
