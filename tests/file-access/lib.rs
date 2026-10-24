// Exercise production grant logic; native IPC is replaced by the unit-test backend.
extern crate self as ohos_fileshare_binding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyInfo {
    pub uri: String,
    pub operation_mode: u32,
}

#[derive(Debug)]
pub struct PolicyErrorResult {
    pub uri: String,
}

pub fn persist_permission(_: &[PolicyInfo]) -> std::io::Result<Vec<PolicyErrorResult>> {
    Err(std::io::Error::other("Unexpected native file grant"))
}

pub fn activate_permission(_: &[PolicyInfo]) -> std::io::Result<Vec<PolicyErrorResult>> {
    Err(std::io::Error::other("Unexpected native file activation"))
}

pub fn check_persistent_permission(_: &[PolicyInfo]) -> std::io::Result<Vec<bool>> {
    Err(std::io::Error::other("Unexpected native file query"))
}

#[allow(dead_code)]
#[path = "../../src/ohos/file_access.rs"]
mod file_access;
#[allow(dead_code)]
#[path = "../../src/ohos/platform_options.rs"]
mod platform_options;
#[allow(dead_code)]
#[path = "../../src/ohos/presentation.rs"]
mod presentation;
