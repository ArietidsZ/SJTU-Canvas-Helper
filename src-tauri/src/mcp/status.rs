//! Read-only runtime observations. No token, owner, URL or raw error is exposed.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum McpState {
    Stopped,
    Running,
    Stopping,
    Exited,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpStatus {
    pub(crate) state: McpState,
    pub(crate) configured_enabled: bool,
    pub(crate) configured_port: u16,
    pub(crate) listening_port: Option<u16>,
    pub(crate) headless: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_schema_is_a_small_credential_free_snapshot() {
        let value = serde_json::to_value(McpStatus {
            state: McpState::Running,
            configured_enabled: false,
            configured_port: 3100,
            listening_port: Some(3200),
            headless: true,
        }).unwrap();
        assert_eq!(value, serde_json::json!({
            "state": "running", "configured_enabled": false,
            "configured_port": 3100, "listening_port": 3200, "headless": true,
        }));
        for (state, expected) in [
            (McpState::Stopped, "stopped"),
            (McpState::Stopping, "stopping"),
            (McpState::Exited, "exited"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), expected);
        }
    }
}
