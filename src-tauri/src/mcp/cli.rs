//! Process-only CLI options. This module deliberately depends on std alone so
//! argument validation can run before loading or recovering account state.
use std::{ffi::OsString, io};

/// Help is side-effect free and takes precedence over other MCP arguments.
/// A plain desktop `--help` keeps the application's existing argument behavior.
pub(crate) fn help_requested(args: &[OsString]) -> bool {
    args.iter().any(|arg| arg == "--mcp")
        && args.iter().any(|arg| arg == "--help" || arg == "-h")
}

pub(crate) const HELP: &str = "SJTU Canvas Helper: MCP server (preview)\n\nUsage:\n  <executable> --mcp [--mcp-port <1-65535>]\n  <executable> --mcp --help\n\nFirst configure your account in the desktop app, then fully quit it.\nThe server uses the saved account and port; --mcp-port overrides only this run.\nConnect a Streamable HTTP client to http://127.0.0.1:<port>/mcp.\n\nKeep this process running. On Unix, Ctrl+C or SIGTERM requests graceful shutdown.\nDo not remove configuration lock files or run the desktop app against the same\nconfiguration directory. No service or startup item is installed.\n\nHeadless chat, explain_file and test_token are disabled. Other local processes\ncan access the listener: use a trusted machine and do not expose it publicly.\nWindows console shutdown behavior remains unverified. See docs/mcp.md.\n";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HeadlessOptions {
    pub(crate) port: Option<u16>,
}

impl HeadlessOptions {
    pub(crate) fn parse(
        args: impl IntoIterator<Item = OsString>,
    ) -> io::Result<Option<Self>> {
        let args: Vec<_> = args.into_iter().collect();
        if !args.iter().any(|arg| arg == "--mcp") {
            // Keep the desktop application's existing argument behavior, but
            // never silently interpret a misspelled MCP invocation as GUI mode.
            if args.iter().any(|arg| arg.to_string_lossy().starts_with("--mcp")) {
                return Err(invalid("MCP options require an explicit --mcp"));
            }
            return Ok(None);
        }
        let mut options = Self::default();
        let mut mode_seen = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.to_str() {
                Some("--mcp") if !mode_seen => mode_seen = true,
                Some("--mcp-port") if options.port.is_none() => {
                    let value = iter.next().and_then(|value| value.to_str());
                    options.port = Some(value
                        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
                        .and_then(|value| value.parse::<u16>().ok())
                        .filter(|port| *port != 0)
                        .ok_or_else(|| invalid("--mcp-port requires an integer between 1 and 65535"))?);
                }
                Some("--mcp-allow-ai") => {
                    return Err(invalid("--mcp-allow-ai is not supported; headless AI tools are disabled"));
                }
                _ => return Err(invalid("Unknown, duplicate, or invalid MCP argument")),
            }
        }
        Ok(Some(options))
    }

    pub(crate) fn effective_port(self, saved: u16) -> io::Result<u16> {
        let port = self.port.unwrap_or(saved);
        if port == 0 {
            return Err(invalid("MCP port must be between 1 and 65535; use --mcp-port to override it"));
        }
        Ok(port)
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{message}. Use --mcp --help for usage."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> io::Result<Option<HeadlessOptions>> {
        HeadlessOptions::parse(args.iter().map(|arg| OsString::from(*arg)))
    }

    #[test]
    fn explicit_mcp_help_is_available_without_parsing_options() {
        for args in [
            &["--mcp", "--help"][..], &["-h", "--mcp"],
            &["--mcp", "--mcp-port", "bad", "--help"],
        ] {
            assert!(help_requested(&args.iter().map(OsString::from).collect::<Vec<_>>()));
        }
        for args in [&[][..], &["--help"], &["--mcp-help"], &["--mcp"]] {
            assert!(!help_requested(&args.iter().map(OsString::from).collect::<Vec<_>>()));
        }
    }

    #[test]
    fn invalid_commands_point_to_usage_without_echoing_arguments() {
        let error = parse(&["--mcp", "--mcp-port", "canary-private-value"]).unwrap_err();
        assert!(error.to_string().contains("--mcp --help"));
        assert!(!error.to_string().contains("canary-private-value"));
    }

    #[test]
    fn help_explains_connection_shutdown_and_trust_boundary() {
        for expected in ["Streamable HTTP", "127.0.0.1", "Ctrl+C", "fully quit", "disabled", "do not expose"] {
            assert!(HELP.contains(expected), "{expected}");
        }
    }

    #[test]
    fn no_arguments_and_non_mcp_arguments_keep_gui_behavior() {
        for args in [&[][..], &["-psn_0_1234"], &["--desktop-option", "file.txt"]] {
            assert_eq!(parse(args).unwrap(), None);
        }
    }

    #[test]
    fn headless_requires_explicit_mode() {
        for args in [
            &["--mcp-port", "3100"][..], &["--mcp-allow-ai"],
            &["--mcp-unknown"], &["--mcp=1"], &["--mcpx"],
        ] {
            assert_eq!(parse(args).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        assert_eq!(parse(&["--mcp"]).unwrap(), Some(HeadlessOptions::default()));
    }

    #[test]
    fn port_bounds_and_order_are_explicit() {
        for port in [1, 3100, 65535] {
            let value = port.to_string();
            for args in [["--mcp", "--mcp-port", &value], ["--mcp-port", &value, "--mcp"]] {
                assert_eq!(parse(&args).unwrap(), Some(HeadlessOptions { port: Some(port) }));
            }
        }
    }

    #[test]
    fn malformed_port_is_not_coerced() {
        for port in ["", "0", "65536", "-1", "+1", "1.0", " 3100", "3100 ", "--mcp", "１２"] {
            assert!(parse(&["--mcp", "--mcp-port", port]).is_err(), "{port}");
        }
        assert!(parse(&["--mcp", "--mcp-port"]).is_err());
    }

    #[test]
    fn unknown_duplicate_and_ai_options_are_rejected() {
        for args in [
            &["--mcp", "--mcp"][..], &["--mcp", "--unknown"],
            &["--mcp", "--mcp-port", "3100", "--mcp-port", "3101"],
            &["--mcp", "--mcp-port=3100"], &["--mcp", "--mcp-allow-ai"],
            &["--mcp", "--", "file"], &["--mcp", "file"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn saved_port_is_used_without_a_process_override() {
        assert_eq!(HeadlessOptions::default().effective_port(4567).unwrap(), 4567);
        assert!(HeadlessOptions::default().effective_port(0).is_err());
    }

    #[test]
    fn process_override_does_not_depend_on_saved_port() {
        let options = HeadlessOptions { port: Some(3210) };
        assert_eq!(options.effective_port(4567).unwrap(), 3210);
        assert_eq!(options.effective_port(0).unwrap(), 3210);
        assert!(HeadlessOptions { port: Some(0) }.effective_port(3100).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_arguments_are_rejected_in_headless_mode_without_panicking() {
        use std::os::unix::ffi::OsStringExt;
        let invalid = OsString::from_vec(vec![0xff]);
        assert!(HeadlessOptions::parse([OsString::from("--mcp"), invalid.clone()]).is_err());
        assert!(HeadlessOptions::parse([OsString::from("--mcp"), OsString::from("--mcp-port"), invalid.clone()]).is_err());
        assert_eq!(HeadlessOptions::parse([invalid]).unwrap(), None);
    }
}
