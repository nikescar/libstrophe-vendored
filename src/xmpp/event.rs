//! Transport-independent event type shared by the WebSocket and BOSH clients.

/// Event delivered to a WebSocket or BOSH connection handler.
///
/// The handler returns `true` to keep the session, `false` to close it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmppEvent {
    /// SASL authentication and resource binding finished; the session is usable.
    Connect,
    /// A top-level stanza received after [`XmppEvent::Connect`].
    Stanza(String),
    /// The session ended (closed by either side).
    Disconnect,
    /// A non-fatal problem, or the cause of a fatal one before `Disconnect`.
    Error(String),
}
