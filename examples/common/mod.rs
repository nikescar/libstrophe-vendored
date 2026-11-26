//! Shared configuration utilities for examples
//!
//! Provides environment variable support with command-line argument fallback.

use std::env;

/// XMPP connection configuration
#[derive(Debug, Clone)]
pub struct XmppConfig {
    pub jid: String,
    pub password: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub ws_url: Option<String>,
}

impl XmppConfig {
    /// Create configuration from environment variables and command-line arguments
    ///
    /// Priority (highest to lowest):
    /// 1. Environment variables
    /// 2. Command-line arguments
    /// 3. Defaults (if any)
    ///
    /// Environment variables:
    /// - `XMPP_JID` - Jabber ID
    /// - `XMPP_PASS` - Password
    /// - `XMPP_HOST` - Server hostname (optional)
    /// - `XMPP_PORT` - Server port (optional)
    /// - `XMPP_WS_URL` - WebSocket URL (for WebSocket examples)
    pub fn from_env_and_args(
        cli_jid: Option<&str>,
        cli_pass: Option<&str>,
        cli_host: Option<&str>,
        cli_port: Option<u16>,
        cli_ws_url: Option<&str>,
    ) -> Self {
        // JID: ENV > CLI
        let jid = env::var("XMPP_JID")
            .ok()
            .or_else(|| cli_jid.map(String::from))
            .unwrap_or_default();

        // Password: ENV > CLI
        let password = env::var("XMPP_PASS")
            .ok()
            .or_else(|| cli_pass.map(String::from))
            .unwrap_or_default();

        // Host: ENV > CLI
        let host = env::var("XMPP_HOST")
            .ok()
            .or_else(|| cli_host.map(String::from));

        // Port: ENV > CLI
        let port = env::var("XMPP_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .or(cli_port);

        // WebSocket URL: ENV > CLI
        let ws_url = env::var("XMPP_WS_URL")
            .ok()
            .or_else(|| cli_ws_url.map(String::from));

        Self { jid, password, host, port, ws_url }
    }

    /// Check if configuration is valid (has JID and password)
    pub fn is_valid(&self) -> bool {
        !self.jid.is_empty() && !self.password.is_empty()
    }

    /// Get display-safe password (masked)
    pub fn password_masked(&self) -> String {
        if self.password.is_empty() {
            String::new()
        } else {
            "***".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_cli_only() {
        env::remove_var("XMPP_JID");
        env::remove_var("XMPP_PASS");

        let config = XmppConfig::from_env_and_args(
            Some("cli@example.com"),
            Some("clipass"),
            None,
            None,
            None,
        );

        assert_eq!(config.jid, "cli@example.com");
        assert_eq!(config.password, "clipass");
        assert!(config.is_valid());
    }

    #[test]
    fn test_config_env_priority() {
        env::set_var("XMPP_JID", "env@example.com");
        env::set_var("XMPP_PASS", "envpass");

        let config = XmppConfig::from_env_and_args(
            Some("cli@example.com"),
            Some("clipass"),
            None,
            None,
            None,
        );

        // Environment variables should take priority
        assert_eq!(config.jid, "env@example.com");
        assert_eq!(config.password, "envpass");

        env::remove_var("XMPP_JID");
        env::remove_var("XMPP_PASS");
    }

    #[test]
    fn test_config_ws_url() {
        env::set_var("XMPP_WS_URL", "wss://env.example.com/ws");

        let config = XmppConfig::from_env_and_args(
            Some("user@example.com"),
            Some("pass"),
            None,
            None,
            Some("wss://cli.example.com/ws"),
        );

        assert_eq!(config.ws_url, Some("wss://env.example.com/ws".to_string()));

        env::remove_var("XMPP_WS_URL");
    }

    #[test]
    fn test_config_port_parsing() {
        env::set_var("XMPP_PORT", "5222");

        let config = XmppConfig::from_env_and_args(
            Some("user@example.com"),
            Some("pass"),
            None,
            Some(5223),
            None,
        );

        assert_eq!(config.port, Some(5222));

        env::remove_var("XMPP_PORT");
    }

    #[test]
    fn test_password_masked() {
        let config = XmppConfig::from_env_and_args(
            Some("user@example.com"),
            Some("secret123"),
            None,
            None,
            None,
        );

        assert_eq!(config.password_masked(), "***");
    }
}
