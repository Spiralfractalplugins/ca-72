//! What SETTINGS: AGENTS copies to set an agent's client up (docs/mcp.md): the bridge that
//! came with the application, beside it, as Claude Code's command or as the `mcpServers`
//! entry other clients (Claude Desktop among them) read from their JSON settings.

use serde_json::json;
use std::path::{Path, PathBuf};

/// The bridge's file name.
pub const BRIDGE: &str = if cfg!(windows) {
    "ca72-mcp.exe"
} else {
    "ca72-mcp"
};

/// The bridge beside this application, if it is there (in the tests, the one a test has put
/// there).
pub fn bridge() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_BRIDGE.with(|b| b.borrow().clone()) {
        return Some(path);
    }
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(BRIDGE);
    path.is_file().then_some(path)
}

#[cfg(test)]
thread_local! {
    /// The bridge a test has put beside the application, on this thread.
    pub(crate) static TEST_BRIDGE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Claude Code's command adding the bridge for every project of the user's.
pub fn claude_code(bridge: &Path) -> String {
    format!(
        "claude mcp add --scope user ca-72 -- \"{}\"",
        bridge.display()
    )
}

/// The bridge as an MCP client's JSON settings name it.
pub fn json(bridge: &Path) -> String {
    let entry = json!({"mcpServers": {"ca-72": {"command": bridge, "args": []}}});
    serde_json::to_string_pretty(&entry).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path with spaces and backslashes (Windows' Program Files) survives in both.
    #[test]
    fn setups_carry_the_path_whole() {
        let p = Path::new(r"C:\Program Files\CA-72\ca72-mcp.exe");
        assert_eq!(
            claude_code(p),
            r#"claude mcp add --scope user ca-72 -- "C:\Program Files\CA-72\ca72-mcp.exe""#
        );
        let v: serde_json::Value = serde_json::from_str(&json(p)).expect("json");
        assert_eq!(
            v["mcpServers"]["ca-72"]["command"],
            r"C:\Program Files\CA-72\ca72-mcp.exe"
        );
        assert_eq!(v["mcpServers"]["ca-72"]["args"], json!([]));
    }
}
