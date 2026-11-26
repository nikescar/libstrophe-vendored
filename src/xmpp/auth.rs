//! Sans-I/O XMPP client login (SASL PLAIN, stream restart, resource binding)
//! shared by the WebSocket and BOSH transports.
//!
//! The transport feeds every received top-level element to [`Auth::on_stanza`]
//! and performs the returned [`Step`].

use super::error::{Error, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

/// What the transport must do after a received element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Nothing to do yet; wait for the next element.
    Wait,
    /// Send this element.
    Send(String),
    /// SASL succeeded: restart the stream (WebSocket `<open/>`, BOSH `xmpp:restart`).
    Restart,
    /// Session ready with this full JID.
    Bound(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Features,
    Sasl,
    BindFeatures,
    Binding { session_required: bool },
    Session,
    Done,
}

pub(crate) struct Auth {
    state: State,
    username: String,
    password: String,
    resource: String,
    jid: String,
}

impl Auth {
    pub(crate) fn new(jid: &str, password: &str, resource: &str) -> Result<Self> {
        Ok(Self {
            state: State::Features,
            username: extract_username(jid).ok_or(Error::InvalidOp)?,
            password: password.to_owned(),
            resource: resource.to_owned(),
            jid: String::new(),
        })
    }

    pub(crate) fn is_bound(&self) -> bool {
        self.state == State::Done
    }

    pub(crate) fn on_stanza(&mut self, xml: &str) -> Result<Step> {
        match self.state {
            State::Features if is_features(xml) => {
                if !supports_sasl_plain(xml) {
                    return Err(Error::Auth); // shortcut: PLAIN only, add SCRAM when a server needs it
                }
                self.state = State::Sasl;
                Ok(Step::Send(create_sasl_plain_auth(&self.username, &self.password)))
            }
            State::Sasl if xml.trim_start().starts_with("<success") => {
                self.state = State::BindFeatures;
                Ok(Step::Restart)
            }
            State::Sasl if xml.trim_start().starts_with("<failure") => Err(Error::Auth),
            State::BindFeatures if is_features(xml) => {
                if !supports_bind(xml) {
                    return Err(Error::Protocol);
                }
                self.state = State::Binding { session_required: session_required(xml) };
                Ok(Step::Send(create_bind_request(Some(&self.resource))))
            }
            State::Binding { session_required } if has_id(xml, "bind-1") => {
                if is_iq_error(xml) {
                    return Err(Error::Auth);
                }
                self.jid = parse_bound_jid(xml).ok_or(Error::Protocol)?;
                if session_required {
                    self.state = State::Session;
                    return Ok(Step::Send(
                        "<iq type='set' id='sess-1' xmlns='jabber:client'><session xmlns='urn:ietf:params:xml:ns:xmpp-session'/></iq>".into(),
                    ));
                }
                self.state = State::Done;
                Ok(Step::Bound(self.jid.clone()))
            }
            State::Session if has_id(xml, "sess-1") => {
                if is_iq_error(xml) {
                    return Err(Error::Auth);
                }
                self.state = State::Done;
                Ok(Step::Bound(self.jid.clone()))
            }
            _ => Ok(Step::Wait),
        }
    }
}

/// Domain part of a JID (`user@domain/resource` -> `domain`).
pub(crate) fn extract_domain_from_jid(jid: &str) -> Result<String> {
    jid.split('@')
        .nth(1)
        .and_then(|s| s.split('/').next())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or(Error::InvalidOp)
}

/// Local part of a JID (`user@domain/resource` -> `user`).
pub(crate) fn extract_username(jid: &str) -> Option<String> {
    jid.split('@').next().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// SASL PLAIN `<auth/>`: base64(`\0username\0password`). Only send over TLS.
pub(crate) fn create_sasl_plain_auth(username: &str, password: &str) -> String {
    let encoded = BASE64.encode(format!("\0{username}\0{password}").as_bytes());
    format!("<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'>{encoded}</auth>")
}

/// `<stream:features>` or unprefixed `<features>` (RFC 7395 / XEP-0206 both occur).
fn is_features(xml: &str) -> bool {
    let t = xml.trim_start();
    t.starts_with("<stream:features") || t.starts_with("<features")
}

pub(crate) fn supports_sasl_plain(features: &str) -> bool {
    features.contains("<mechanism>PLAIN</mechanism")
}

pub(crate) fn supports_bind(features: &str) -> bool {
    features.contains("xmlns='urn:ietf:params:xml:ns:xmpp-bind'")
        || features.contains(r#"xmlns="urn:ietf:params:xml:ns:xmpp-bind""#)
}

/// RFC 3921 session IQ is needed only if advertised without `<optional/>`.
fn session_required(features: &str) -> bool {
    let Some(pos) = features.find("urn:ietf:params:xml:ns:xmpp-session") else {
        return false;
    };
    let rest = &features[pos..];
    let end = rest.find("</session>").unwrap_or(rest.find("/>").unwrap_or(rest.len()));
    !rest[..end].contains("<optional")
}

pub(crate) fn create_bind_request(resource: Option<&str>) -> String {
    let resource_xml = resource
        .map(|r| format!("<resource>{}</resource>", escape(r)))
        .unwrap_or_default();
    format!("<iq type='set' id='bind-1' xmlns='jabber:client'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'>{resource_xml}</bind></iq>")
}

pub(crate) fn parse_bound_jid(iq: &str) -> Option<String> {
    let start = iq.find("<jid>")? + 5;
    let end = iq[start..].find("</jid>")? + start;
    Some(iq[start..end].to_owned())
}

fn has_id(xml: &str, id: &str) -> bool {
    xml.contains(&format!("id='{id}'")) || xml.contains(&format!(r#"id="{id}""#))
}

fn is_iq_error(xml: &str) -> bool {
    xml.contains("type='error'") || xml.contains(r#"type="error""#)
}

/// XML-escape text or attribute content.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEATURES_PLAIN: &str = "<stream:features xmlns:stream='http://etherx.jabber.org/streams'><mechanisms xmlns='urn:ietf:params:xml:ns:xmpp-sasl'><mechanism>SCRAM-SHA-1</mechanism><mechanism>PLAIN</mechanism></mechanisms></stream:features>";
    const FEATURES_BIND: &str = "<features xmlns='http://etherx.jabber.org/streams'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'/></features>";
    const BIND_RESULT: &str = "<iq type='result' id='bind-1'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'><jid>juliet@example.com/r</jid></bind></iq>";

    #[test]
    fn full_login_sequence() {
        let mut a = Auth::new("juliet@example.com", "pw", "r").unwrap();
        assert!(matches!(a.on_stanza(FEATURES_PLAIN).unwrap(), Step::Send(s) if s.contains("mechanism='PLAIN'")));
        assert_eq!(a.on_stanza("<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>").unwrap(), Step::Restart);
        assert!(matches!(a.on_stanza(FEATURES_BIND).unwrap(), Step::Send(s) if s.contains("<resource>r</resource>")));
        assert!(!a.is_bound());
        assert_eq!(a.on_stanza(BIND_RESULT).unwrap(), Step::Bound("juliet@example.com/r".into()));
        assert!(a.is_bound());
    }

    #[test]
    fn required_session_iq_is_sent() {
        let mut a = Auth::new("juliet@example.com", "pw", "r").unwrap();
        a.on_stanza(FEATURES_PLAIN).unwrap();
        a.on_stanza("<success/>").unwrap();
        let f = "<stream:features><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'/><session xmlns='urn:ietf:params:xml:ns:xmpp-session'/></stream:features>";
        a.on_stanza(f).unwrap();
        assert!(matches!(a.on_stanza(BIND_RESULT).unwrap(), Step::Send(s) if s.contains("sess-1")));
        assert_eq!(a.on_stanza("<iq type='result' id='sess-1'/>").unwrap(), Step::Bound("juliet@example.com/r".into()));
    }

    #[test]
    fn optional_session_is_skipped() {
        let f = "<stream:features><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'/><session xmlns='urn:ietf:params:xml:ns:xmpp-session'><optional/></session></stream:features>";
        assert!(!session_required(f));
    }

    #[test]
    fn sasl_failure_and_missing_plain_are_auth_errors() {
        let mut a = Auth::new("juliet@example.com", "pw", "r").unwrap();
        a.on_stanza(FEATURES_PLAIN).unwrap();
        assert_eq!(a.on_stanza("<failure xmlns='urn:ietf:params:xml:ns:xmpp-sasl'><not-authorized/></failure>"), Err(Error::Auth));

        let mut a = Auth::new("juliet@example.com", "pw", "r").unwrap();
        let scram_only = "<stream:features><mechanisms><mechanism>SCRAM-SHA-1</mechanism></mechanisms></stream:features>";
        assert_eq!(a.on_stanza(scram_only), Err(Error::Auth));
    }

    #[test]
    fn unrelated_elements_wait() {
        let mut a = Auth::new("juliet@example.com", "pw", "r").unwrap();
        assert_eq!(a.on_stanza("<message/>").unwrap(), Step::Wait);
    }

    #[test]
    fn sasl_plain_encoding() {
        let auth = create_sasl_plain_auth("juliet", "r0m30myr0m30");
        let start = auth.find('>').unwrap() + 1;
        let end = auth.find("</auth>").unwrap();
        assert_eq!(BASE64.decode(&auth[start..end]).unwrap(), b"\0juliet\0r0m30myr0m30");
    }

    #[test]
    fn jid_parts() {
        assert_eq!(extract_username("user@example.com/res"), Some("user".into()));
        assert_eq!(extract_domain_from_jid("user@example.com/res").unwrap(), "example.com");
        assert!(extract_domain_from_jid("invalid").is_err());
        assert!(extract_domain_from_jid("user@").is_err());
    }

    #[test]
    fn bind_helpers() {
        assert!(supports_bind(FEATURES_BIND));
        assert!(!supports_bind(FEATURES_PLAIN));
        assert!(!create_bind_request(None).contains("<resource>"));
        assert_eq!(parse_bound_jid("<iq><bind/></iq>"), None);
        assert_eq!(escape(r#"a<b>&"c'"#), "a&lt;b&gt;&amp;&quot;c&apos;");
    }
}
