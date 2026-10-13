//! Pure-logic helpers for locating `deckcraft-cli`, detecting the runtime package kind,
//! and generating Model Context Protocol (MCP) configuration and registration commands.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    Native,
    AppImage,
    Flatpak,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliLaunch {
    pub program: String,
    pub args: Vec<String>,
    pub temporary: bool,
}

/// Detect whether DeckCraft is running as a native install, AppImage, or Flatpak.
pub fn detect_package_kind(appimage_env: Option<&str>, flatpak_id_env: Option<&str>, flatpak_info_exists: bool) -> PackageKind {
    if appimage_env.is_some_and(|s| !s.is_empty()) {
        PackageKind::AppImage
    } else if flatpak_id_env.is_some_and(|s| !s.is_empty()) || flatpak_info_exists {
        PackageKind::Flatpak
    } else {
        PackageKind::Native
    }
}

/// Work out how an external agent or process should invoke `deckcraft-cli`.
///
/// `current_exe` is treated as a plain string rather than `std::path::Path` so paths
/// can be cross-platform tested regardless of host OS.
pub fn cli_launch(os: Os, kind: PackageKind, current_exe: &str) -> CliLaunch {
    if kind == PackageKind::Flatpak {
        return CliLaunch {
            program: "flatpak".to_string(),
            args: vec!["run".to_string(), "--command=deckcraft-cli".to_string(), "ai.storyteller.deckcraft".to_string()],
            temporary: false,
        };
    }

    let (sep, bin_name) = match os {
        Os::Windows => ('\\', "deckcraft-cli.exe"),
        Os::MacOs | Os::Linux => ('/', "deckcraft-cli"),
    };

    let program = match current_exe.rfind(sep) {
        Some(idx) => format!("{}{}{}", &current_exe[..idx], sep, bin_name),
        None => bin_name.to_string(),
    };

    CliLaunch { program, args: Vec::new(), temporary: kind == PackageKind::AppImage }
}

/// Arguments to launch `deckcraft-cli` in MCP mode (headless or connected to a port).
pub fn mcp_args(launch: &CliLaunch, connect_port: Option<u16>) -> Vec<String> {
    let mut args = launch.args.clone();
    args.push("mcp".to_string());
    if let Some(port) = connect_port {
        args.push("--connect".to_string());
        args.push(port.to_string());
    }
    args
}

/// Pretty-printed JSON MCP server configuration, for MCP clients that read a JSON config (for example Claude Desktop).
pub fn mcp_json_config(launch: &CliLaunch, connect_port: Option<u16>) -> String {
    let args = mcp_args(launch, connect_port);
    let val = serde_json::json!({
        "mcpServers": {
            "deckcraft": {
                "command": launch.program,
                "args": args
            }
        }
    });
    serde_json::to_string_pretty(&val).unwrap_or_default()
}

/// Shell quotation helper for a command argument.
pub fn quote_arg(os: Os, s: &str) -> String {
    match os {
        Os::Windows => {
            if !s.contains(' ') && !s.contains('"') && !s.contains('\'') {
                s.to_string()
            } else {
                format!("\"{s}\"")
            }
        }
        Os::MacOs | Os::Linux => {
            let is_safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-');
            if !s.is_empty() && s.chars().all(is_safe) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
        }
    }
}

/// Single-line CLI registration command for Claude Desktop / CLI (`claude mcp add deckcraft -- ...`).
pub fn registration_command(os: Os, launch: &CliLaunch, connect_port: Option<u16>) -> String {
    let mut parts = vec!["claude".to_string(), "mcp".to_string(), "add".to_string(), "deckcraft".to_string(), "--".to_string()];
    parts.push(quote_arg(os, &launch.program));
    for arg in mcp_args(launch, connect_port) {
        parts.push(quote_arg(os, &arg));
    }
    parts.join(" ")
}

/// Generate both the CLI registration command and the JSON MCP configuration.
pub fn setup_texts(os: Os, kind: PackageKind, exe: &str, port: Option<u16>) -> (String, String) {
    let launch = cli_launch(os, kind, exe);
    (registration_command(os, &launch, port), mcp_json_config(&launch, port))
}

/// Generate registration command and MCP JSON for an explicit program path and args.
pub fn setup_texts_for_program(os: Os, program: &str, args: Vec<String>, port: Option<u16>) -> (String, String) {
    let launch = CliLaunch { program: program.to_string(), args, temporary: false };
    (registration_command(os, &launch, port), mcp_json_config(&launch, port))
}

/// Locate potential `deckcraft-cli` candidate paths on the filesystem.
pub fn cli_candidates(os: Os, kind: PackageKind, current_exe: &str, path_env: Option<&str>) -> Vec<String> {
    if kind == PackageKind::Flatpak {
        return Vec::new();
    }

    let mut out: Vec<String> = Vec::new();

    // 1. Co-located binary
    let colocated = cli_launch(os, kind, current_exe).program;
    if !colocated.is_empty() {
        out.push(colocated);
    }

    // 2. Extra standard macOS locations
    if os == Os::MacOs {
        for mac_path in ["/usr/local/bin/deckcraft-cli", "/opt/homebrew/bin/deckcraft-cli"] {
            if !out.iter().any(|p| p == mac_path) {
                out.push(mac_path.to_string());
            }
        }
    }

    // 3. Entries from PATH
    let (path_sep, bin_name, file_sep) = match os {
        Os::Windows => (';', "deckcraft-cli.exe", '\\'),
        Os::MacOs | Os::Linux => (':', "deckcraft-cli", '/'),
    };

    if let Some(paths) = path_env {
        for entry in paths.split(path_sep) {
            let trimmed = entry.trim();
            if !trimmed.is_empty() {
                let candidate = if trimmed.ends_with(file_sep) { format!("{trimmed}{bin_name}") } else { format!("{trimmed}{file_sep}{bin_name}") };
                if !out.iter().any(|p| p == &candidate) {
                    out.push(candidate);
                }
            }
        }
    }

    out
}

/// Return the first candidate that exists as a file on disk.
pub fn first_existing(candidates: &[String]) -> Option<String> {
    candidates.iter().find(|c| std::path::Path::new(c).is_file()).cloned()
}

/// Probe a CLI executable for its version string by invoking `--version`.
pub fn cli_version(launch: &CliLaunch, timeout: std::time::Duration) -> Result<String, String> {
    let mut cmd = std::process::Command::new(&launch.program);
    cmd.args(&launch.args);
    cmd.arg("--version");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return match e.kind() {
                std::io::ErrorKind::NotFound => Err("not found".to_string()),
                std::io::ErrorKind::PermissionDenied => Err("not executable".to_string()),
                _ => Err(e.to_string()),
            };
        }
    };

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let code_str = status.code().map(|c| c.to_string()).unwrap_or_else(|| "terminated".to_string());
                    return Err(format!("exited with status {code_str}"));
                }
                let mut stdout_str = String::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    let _ = out.read_to_string(&mut stdout_str);
                }
                for line in stdout_str.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        return Ok(trimmed.to_string());
                    }
                }
                return Ok(String::new());
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let secs = timeout.as_secs();
                    return Err(format!("did not respond within {secs} seconds"));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_package_kind() {
        assert_eq!(detect_package_kind(Some("/path/to/app.AppImage"), None, false), PackageKind::AppImage);
        assert_eq!(detect_package_kind(Some("/path/to/app.AppImage"), Some("ai.storyteller.deckcraft"), true), PackageKind::AppImage);
        assert_eq!(detect_package_kind(None, Some("ai.storyteller.deckcraft"), false), PackageKind::Flatpak);
        assert_eq!(detect_package_kind(None, None, true), PackageKind::Flatpak);
        assert_eq!(detect_package_kind(None, None, false), PackageKind::Native);
        assert_eq!(detect_package_kind(Some(""), Some(""), false), PackageKind::Native);
        assert_eq!(detect_package_kind(Some(""), Some("ai.storyteller.deckcraft"), false), PackageKind::Flatpak);
    }

    #[test]
    fn test_cli_launch_windows_native() {
        let launch = cli_launch(Os::Windows, PackageKind::Native, r"C:\Program Files\DeckCraft\deckcraft.exe");
        assert_eq!(launch.program, r"C:\Program Files\DeckCraft\deckcraft-cli.exe");
        assert!(launch.args.is_empty());
        assert!(!launch.temporary);
    }

    #[test]
    fn test_cli_launch_linux_native() {
        let launch = cli_launch(Os::Linux, PackageKind::Native, "/usr/bin/deckcraft");
        assert_eq!(launch.program, "/usr/bin/deckcraft-cli");
        assert!(launch.args.is_empty());
        assert!(!launch.temporary);
    }

    #[test]
    fn test_cli_launch_linux_appimage() {
        let launch = cli_launch(Os::Linux, PackageKind::AppImage, "/tmp/.mount_abc/usr/bin/deckcraft");
        assert_eq!(launch.program, "/tmp/.mount_abc/usr/bin/deckcraft-cli");
        assert!(launch.args.is_empty());
        assert!(launch.temporary);
    }

    #[test]
    fn test_cli_launch_macos_native() {
        let launch = cli_launch(Os::MacOs, PackageKind::Native, "/Applications/DeckCraft.app/Contents/MacOS/DeckCraft");
        assert_eq!(launch.program, "/Applications/DeckCraft.app/Contents/MacOS/deckcraft-cli");
        assert!(launch.args.is_empty());
        assert!(!launch.temporary);
    }

    #[test]
    fn test_cli_launch_linux_flatpak() {
        let launch = cli_launch(Os::Linux, PackageKind::Flatpak, "/app/bin/deckcraft");
        assert_eq!(launch.program, "flatpak");
        assert_eq!(launch.args, vec!["run", "--command=deckcraft-cli", "ai.storyteller.deckcraft"]);
        assert!(!launch.temporary);
    }

    #[test]
    fn test_mcp_args() {
        let native = cli_launch(Os::Linux, PackageKind::Native, "/usr/bin/deckcraft");
        assert_eq!(mcp_args(&native, None), vec!["mcp"]);
        assert_eq!(mcp_args(&native, Some(7990)), vec!["mcp", "--connect", "7990"]);

        let flatpak = cli_launch(Os::Linux, PackageKind::Flatpak, "/app/bin/deckcraft");
        assert_eq!(mcp_args(&flatpak, None), vec!["run", "--command=deckcraft-cli", "ai.storyteller.deckcraft", "mcp"]);
        assert_eq!(mcp_args(&flatpak, Some(7990)), vec!["run", "--command=deckcraft-cli", "ai.storyteller.deckcraft", "mcp", "--connect", "7990"]);
    }

    #[test]
    fn test_registration_command() {
        let win = cli_launch(Os::Windows, PackageKind::Native, r"C:\Program Files\DeckCraft\deckcraft.exe");
        assert_eq!(
            registration_command(Os::Windows, &win, Some(7990)),
            r#"claude mcp add deckcraft -- "C:\Program Files\DeckCraft\deckcraft-cli.exe" mcp --connect 7990"#
        );

        let linux_space = cli_launch(Os::Linux, PackageKind::Native, "/home/user/my apps/deckcraft");
        assert_eq!(registration_command(Os::Linux, &linux_space, None), "claude mcp add deckcraft -- '/home/user/my apps/deckcraft-cli' mcp");
    }

    #[test]
    fn test_mcp_json_config() {
        let win = cli_launch(Os::Windows, PackageKind::Native, r"C:\Program Files\DeckCraft\deckcraft.exe");
        let json_str = mcp_json_config(&win, Some(7990));
        assert!(json_str.contains("\"mcpServers\""));
        assert!(json_str.contains("\"deckcraft\""));
        assert!(json_str.contains(r"C:\\Program Files\\DeckCraft\\deckcraft-cli.exe"));
    }

    #[test]
    fn test_setup_texts_linux_native() {
        let (cmd, json) = setup_texts(Os::Linux, PackageKind::Native, "/usr/bin/deckcraft", None);
        assert_eq!(cmd, "claude mcp add deckcraft -- /usr/bin/deckcraft-cli mcp");
        assert!(json.contains("\"command\": \"/usr/bin/deckcraft-cli\""));
        assert!(json.contains("\"args\": [\n        \"mcp\"\n      ]"));
    }

    #[test]
    fn test_quote_arg_linux() {
        assert_eq!(quote_arg(Os::Linux, "it's"), r"'it'\''s'");
        assert_eq!(quote_arg(Os::Linux, "/home/u/my(app)/deckcraft"), "'/home/u/my(app)/deckcraft'");
        assert_eq!(quote_arg(Os::Linux, "/usr/bin/deckcraft-cli"), "/usr/bin/deckcraft-cli");
        assert_eq!(quote_arg(Os::Linux, ""), "''");
    }

    #[test]
    fn test_cli_candidates_linux_native() {
        let candidates = cli_candidates(Os::Linux, PackageKind::Native, "/usr/bin/deckcraft", Some("/usr/bin:/home/u/bin"));
        assert_eq!(candidates, vec!["/usr/bin/deckcraft-cli".to_string(), "/home/u/bin/deckcraft-cli".to_string()]);
    }

    #[test]
    fn test_cli_candidates_macos() {
        let candidates = cli_candidates(Os::MacOs, PackageKind::Native, "/Applications/DeckCraft.app/Contents/MacOS/DeckCraft", None);
        assert_eq!(
            candidates,
            vec![
                "/Applications/DeckCraft.app/Contents/MacOS/deckcraft-cli".to_string(),
                "/usr/local/bin/deckcraft-cli".to_string(),
                "/opt/homebrew/bin/deckcraft-cli".to_string()
            ]
        );
    }

    #[test]
    fn test_cli_candidates_windows() {
        let candidates = cli_candidates(Os::Windows, PackageKind::Native, r"C:\Program Files\DeckCraft\deckcraft.exe", Some(r"C:\Tools;C:\Bin"));
        assert_eq!(
            candidates,
            vec![
                r"C:\Program Files\DeckCraft\deckcraft-cli.exe".to_string(),
                r"C:\Tools\deckcraft-cli.exe".to_string(),
                r"C:\Bin\deckcraft-cli.exe".to_string()
            ]
        );
    }

    #[test]
    fn test_cli_candidates_flatpak() {
        let candidates = cli_candidates(Os::Linux, PackageKind::Flatpak, "/app/bin/deckcraft", Some("/usr/bin"));
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_cli_version_nonexistent() {
        let launch = CliLaunch { program: "/nonexistent/deckcraft-cli".to_string(), args: Vec::new(), temporary: false };
        let res = cli_version(&launch, std::time::Duration::from_millis(200));
        assert_eq!(res, Err("not found".to_string()));
    }

    #[test]
    fn test_setup_texts_for_program() {
        let (cmd, json) = setup_texts_for_program(Os::Linux, "/custom/deckcraft-cli", vec!["--opt".to_string()], Some(7990));
        assert_eq!(cmd, "claude mcp add deckcraft -- /custom/deckcraft-cli --opt mcp --connect 7990");
        assert!(json.contains("\"command\": \"/custom/deckcraft-cli\""));
        assert!(json.contains("\"--connect\""));
    }
}
