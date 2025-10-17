//! Approval policies for DevIt orchestration
//!
//! Controls which actions require approval based on declarative policies.
//! Supports trusted, untrusted, and on_request approval levels.

use serde_json::Value;

/// Policy decision for an action
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyAction {
    Allow,        // Action is trusted and can proceed
    Deny,         // Action is forbidden
    NeedApproval, // Action requires explicit approval
}

/// Policy evaluator
pub struct Policy;

impl Policy {
    /// Evaluate policy for a given tool and payload
    pub fn eval(tool: &str, payload: &Value) -> PolicyAction {
        match tool {
            "devit_patch_apply" => eval_patch_apply(payload),
            "devit_file_read" => eval_file_read(payload),
            "devit_test_run" => eval_test_run(payload),
            "devit_snapshot" => PolicyAction::Allow, // Snapshots are safe
            "devit_journal_append" => PolicyAction::Allow, // Logging is safe
            _ => PolicyAction::Allow,                // Default allow for unknown tools
        }
    }
}

/// Evaluate patch application policy
fn eval_patch_apply(payload: &Value) -> PolicyAction {
    // Extract path from task payload
    let path = payload
        .get("task")
        .and_then(|t| t.get("path"))
        .and_then(|p| p.as_str());

    if let Some(path) = path {
        // Forbidden paths - immediate deny
        if path.starts_with("/etc")
            || path.starts_with("/var")
            || path.starts_with("/home")
            || path.starts_with("/root")
            || path.starts_with("/sys")
            || path.starts_with("/proc")
        {
            return PolicyAction::Deny;
        }

        // Trusted paths - allow without approval
        if path.starts_with("/workdir/src")
            || path.starts_with("/workdir/tests")
            || path.starts_with("/tmp/devit")
        {
            return PolicyAction::Allow;
        }
    }

    // Default for patch_apply - require approval
    PolicyAction::NeedApproval
}

/// Evaluate file read policy
fn eval_file_read(payload: &Value) -> PolicyAction {
    let path = payload
        .get("task")
        .and_then(|t| t.get("path"))
        .and_then(|p| p.as_str());

    if let Some(path) = path {
        // Sensitive files - deny
        if path.contains("password")
            || path.contains("secret")
            || path.contains("key")
            || path.starts_with("/etc/shadow")
            || path.starts_with("/root/.ssh")
        {
            return PolicyAction::Deny;
        }

        // Check file size limit
        if let Some(size) = payload
            .get("task")
            .and_then(|t| t.get("max_bytes"))
            .and_then(|s| s.as_u64())
        {
            if size > 1_048_576
            // 1MB limit
            {
                return PolicyAction::NeedApproval;
            }
        }
    }

    PolicyAction::Allow
}

/// Evaluate test run policy
fn eval_test_run(payload: &Value) -> PolicyAction {
    // Check timeout
    if let Some(timeout) = payload
        .get("task")
        .and_then(|t| t.get("timeout_secs"))
        .and_then(|t| t.as_u64())
    {
        if timeout > 120
        // 2 minute limit
        {
            return PolicyAction::NeedApproval;
        }
    }

    // Check if test command contains dangerous operations
    if let Some(command) = payload
        .get("task")
        .and_then(|t| t.get("command"))
        .and_then(|c| c.as_str())
    {
        let dangerous_patterns = [
            "rm -rf", "sudo", "chmod +x", "curl", "wget", "network", "docker",
        ];

        for pattern in &dangerous_patterns {
            if command.contains(pattern) {
                return PolicyAction::NeedApproval;
            }
        }
    }

    PolicyAction::Allow
}

/// Create approval request message
pub fn create_approval_request(
    task_id: &str,
    tool: &str,
    original_msg_id: &str,
    details: &Value,
) -> Value {
    serde_json::json!({
        "reason": "on_request",
        "tool": tool,
        "task_id": task_id,
        "original_msg_id": original_msg_id,
        "details": details,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "risk_level": assess_risk_level(tool, details)
    })
}

/// Assess risk level for approval UI
fn assess_risk_level(tool: &str, details: &Value) -> &'static str {
    match tool {
        "devit_patch_apply" => {
            let path = details
                .get("task")
                .and_then(|t| t.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or("");

            if path.contains("main.rs") || path.contains("lib.rs") {
                "medium"
            } else {
                "low"
            }
        }
        "devit_test_run" => {
            let timeout = details
                .get("task")
                .and_then(|t| t.get("timeout_secs"))
                .and_then(|t| t.as_u64())
                .unwrap_or(0);

            if timeout > 60 {
                "medium"
            } else {
                "low"
            }
        }
        _ => "low",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_patch_apply_policy() {
        // Forbidden path
        let payload = json!({
            "task": {
                "action": "devit_patch_apply",
                "path": "/etc/hosts"
            }
        });
        assert_eq!(
            Policy::eval("devit_patch_apply", &payload),
            PolicyAction::Deny
        );

        // Trusted path
        let payload = json!({
            "task": {
                "action": "devit_patch_apply",
                "path": "/workdir/src/main.rs"
            }
        });
        assert_eq!(
            Policy::eval("devit_patch_apply", &payload),
            PolicyAction::Allow
        );

        // Unknown path - needs approval
        let payload = json!({
            "task": {
                "action": "devit_patch_apply",
                "path": "/unknown/file.rs"
            }
        });
        assert_eq!(
            Policy::eval("devit_patch_apply", &payload),
            PolicyAction::NeedApproval
        );
    }

    #[test]
    fn test_file_read_policy() {
        // Sensitive file
        let payload = json!({
            "task": {
                "action": "devit_file_read",
                "path": "/etc/shadow"
            }
        });
        assert_eq!(
            Policy::eval("devit_file_read", &payload),
            PolicyAction::Deny
        );

        // Large file
        let payload = json!({
            "task": {
                "action": "devit_file_read",
                "path": "/workdir/large.txt",
                "max_bytes": 2000000
            }
        });
        assert_eq!(
            Policy::eval("devit_file_read", &payload),
            PolicyAction::NeedApproval
        );

        // Normal file
        let payload = json!({
            "task": {
                "action": "devit_file_read",
                "path": "/workdir/small.txt"
            }
        });
        assert_eq!(
            Policy::eval("devit_file_read", &payload),
            PolicyAction::Allow
        );
    }

    #[test]
    fn test_test_run_policy() {
        // Dangerous command
        let payload = json!({
            "task": {
                "action": "devit_test_run",
                "command": "rm -rf /"
            }
        });
        assert_eq!(
            Policy::eval("devit_test_run", &payload),
            PolicyAction::NeedApproval
        );

        // Long timeout
        let payload = json!({
            "task": {
                "action": "devit_test_run",
                "timeout_secs": 300
            }
        });
        assert_eq!(
            Policy::eval("devit_test_run", &payload),
            PolicyAction::NeedApproval
        );

        // Safe test
        let payload = json!({
            "task": {
                "action": "devit_test_run",
                "command": "cargo test",
                "timeout_secs": 60
            }
        });
        assert_eq!(
            Policy::eval("devit_test_run", &payload),
            PolicyAction::Allow
        );
    }

    #[test]
    fn test_risk_assessment() {
        let details = json!({
            "task": {
                "path": "/workdir/src/main.rs"
            }
        });
        assert_eq!(assess_risk_level("devit_patch_apply", &details), "medium");

        let details = json!({
            "task": {
                "timeout_secs": 90
            }
        });
        assert_eq!(assess_risk_level("devit_test_run", &details), "medium");
    }
}
