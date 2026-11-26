//! XEP-0124 / XEP-0206 `<body/>` wrapper: request builders and response parser.

use crate::xmpp::auth::escape;
use crate::xmpp::error::{Error, Result};

pub(crate) const NS_HTTPBIND: &str = "http://jabber.org/protocol/httpbind";
pub(crate) const NS_XBOSH: &str = "urn:xmpp:xbosh";
const UNAVAILABLE: &str = "<presence type='unavailable' xmlns='jabber:client'/>";

/// Session creation request (XEP-0124 §7, XEP-0206 §5).
pub(crate) fn session_create(rid: u64, to: &str, wait: u32, hold: u32) -> String {
    format!(
        "<body content='text/xml; charset=utf-8' hold='{hold}' rid='{rid}' to='{}' wait='{wait}' ver='1.6' xml:lang='en' xmpp:version='1.0' xmlns='{NS_HTTPBIND}' xmlns:xmpp='{NS_XBOSH}'/>",
        escape(to)
    )
}

/// Request carrying zero (an empty poll) or more stanzas.
pub(crate) fn payload(rid: u64, sid: &str, stanzas: &[String]) -> String {
    let sid = escape(sid);
    if stanzas.is_empty() {
        format!("<body rid='{rid}' sid='{sid}' xmlns='{NS_HTTPBIND}'/>")
    } else {
        format!("<body rid='{rid}' sid='{sid}' xmlns='{NS_HTTPBIND}'>{}</body>", stanzas.concat())
    }
}

/// Stream restart after SASL success (XEP-0206 §5).
pub(crate) fn restart(rid: u64, sid: &str, to: &str) -> String {
    format!(
        "<body rid='{rid}' sid='{}' to='{}' xml:lang='en' xmpp:restart='true' xmlns='{NS_HTTPBIND}' xmlns:xmpp='{NS_XBOSH}'/>",
        escape(sid),
        escape(to)
    )
}

/// Session termination with unavailable presence (XEP-0124 §13).
pub(crate) fn terminate(rid: u64, sid: &str, stanzas: &[String]) -> String {
    format!(
        "<body rid='{rid}' sid='{}' type='terminate' xmlns='{NS_HTTPBIND}'>{}{UNAVAILABLE}</body>",
        escape(sid),
        stanzas.concat()
    )
}

/// Parsed connection-manager response.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct BodyResp {
    attrs: Vec<(String, String)>,
    /// Top-level child elements, verbatim.
    pub children: Vec<String>,
}

impl BodyResp {
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub(crate) fn is_terminate(&self) -> bool {
        self.attr("type") == Some("terminate")
    }
}

/// Parse a `<body/>` response.
///
/// shortcut: hand-rolled scanner (attributes, comments, CDATA, PIs, quoted `>`),
/// not a validating XML parser; switch to one if a server's output trips it.
pub(crate) fn parse_body(xml: &str) -> Result<BodyResp> {
    let mut pos = skip_prolog(xml)?;
    if !xml[pos..].starts_with("<body") {
        return Err(Error::Protocol);
    }
    let (start_end, self_closing) = tag_end(xml, pos)?;
    let attrs = parse_attrs(&xml[pos + "<body".len()..start_end - if self_closing { 2 } else { 1 }])?;
    if self_closing {
        return Ok(BodyResp { attrs, children: Vec::new() });
    }
    pos = start_end;
    let mut children = Vec::new();
    let mut open: Vec<&str> = Vec::new(); // names of unclosed child elements
    let mut start = 0;
    loop {
        let lt = pos + xml[pos..].find('<').ok_or(Error::Protocol)?;
        let rest = &xml[lt..];
        if rest.starts_with("<!--") {
            pos = lt + rest.find("-->").ok_or(Error::Protocol)? + 3;
        } else if rest.starts_with("<![CDATA[") {
            pos = lt + rest.find("]]>").ok_or(Error::Protocol)? + 3;
        } else if rest.starts_with("<?") {
            pos = lt + rest.find("?>").ok_or(Error::Protocol)? + 2;
        } else if rest.starts_with("</") {
            let end = lt + rest.find('>').ok_or(Error::Protocol)? + 1;
            let name = tag_name(&xml[lt + 2..]);
            match open.pop() {
                None if name == "body" => return Ok(BodyResp { attrs, children }),
                Some(o) if o == name => {}
                _ => return Err(Error::Protocol),
            }
            if open.is_empty() {
                children.push(xml[start..end].to_owned());
            }
            pos = end;
        } else {
            let (end, self_closing) = tag_end(xml, lt)?;
            if open.is_empty() {
                start = lt;
            }
            if !self_closing {
                open.push(tag_name(&xml[lt + 1..]));
            } else if open.is_empty() {
                children.push(xml[start..end].to_owned());
            }
            pos = end;
        }
    }
}

fn tag_name(s: &str) -> &str {
    &s[..s.find(|c: char| c.is_whitespace() || c == '/' || c == '>').unwrap_or(s.len())]
}

/// Skip whitespace, an XML declaration and comments before the root element.
fn skip_prolog(xml: &str) -> Result<usize> {
    let mut pos = 0;
    loop {
        pos += xml[pos..].len() - xml[pos..].trim_start().len();
        let rest = &xml[pos..];
        if rest.starts_with("<?") {
            pos += rest.find("?>").ok_or(Error::Protocol)? + 2;
        } else if rest.starts_with("<!--") {
            pos += rest.find("-->").ok_or(Error::Protocol)? + 3;
        } else {
            return Ok(pos);
        }
    }
}

/// Index just past the `>` closing the tag that starts at `lt`, and whether it is `/>`.
fn tag_end(xml: &str, lt: usize) -> Result<(usize, bool)> {
    let mut quote = None;
    for (i, c) in xml[lt..].char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Ok((lt + i + 1, xml[..lt + i].ends_with('/'))),
            _ => {}
        }
    }
    Err(Error::Protocol)
}

fn parse_attrs(mut s: &str) -> Result<Vec<(String, String)>> {
    let mut attrs = Vec::new();
    loop {
        s = s.trim_start();
        if s.is_empty() {
            return Ok(attrs);
        }
        let eq = s.find('=').ok_or(Error::Protocol)?;
        let name = s[..eq].trim();
        let after = s[eq + 1..].trim_start();
        let q = after.chars().next().filter(|c| *c == '"' || *c == '\'').ok_or(Error::Protocol)?;
        let end = after[1..].find(q).ok_or(Error::Protocol)? + 1;
        attrs.push((name.to_owned(), unescape(&after[1..end])));
        s = &after[end + 1..];
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders() {
        let c = session_create(7, "example.com", 60, 1);
        for part in ["rid='7'", "to='example.com'", "wait='60'", "hold='1'", "ver='1.6'", "xmpp:version='1.0'", "xmlns:xmpp='urn:xmpp:xbosh'", "content='text/xml; charset=utf-8'"] {
            assert!(c.contains(part), "{part} missing in {c}");
        }
        assert_eq!(payload(8, "s1", &[]), "<body rid='8' sid='s1' xmlns='http://jabber.org/protocol/httpbind'/>");
        assert!(payload(9, "s1", &["<a/>".into(), "<b/>".into()]).ends_with("><a/><b/></body>"));
        let r = restart(10, "s1", "example.com");
        assert!(r.contains("xmpp:restart='true'") && r.contains("to='example.com'") && r.contains("sid='s1'"));
        let t = terminate(11, "s1", &["<m/>".into()]);
        assert!(t.contains("type='terminate'") && t.contains("<m/><presence type='unavailable'"));
        assert!(session_create(1, "a'b", 1, 1).contains("to='a&apos;b'"));
    }

    #[test]
    fn parse_session_create_response() {
        let xml = r#"<?xml version='1.0'?><body xmlns='http://jabber.org/protocol/httpbind' sid="abc&amp;1" wait='60' requests='2' xmlns:stream='http://etherx.jabber.org/streams'><stream:features><mechanisms xmlns='urn:ietf:params:xml:ns:xmpp-sasl'><mechanism>PLAIN</mechanism></mechanisms></stream:features></body>"#;
        let b = parse_body(xml).unwrap();
        assert_eq!(b.attr("sid"), Some("abc&1"));
        assert_eq!(b.attr("requests"), Some("2"));
        assert_eq!(b.children.len(), 1);
        assert!(b.children[0].starts_with("<stream:features>") && b.children[0].ends_with("</stream:features>"));
    }

    #[test]
    fn parse_multiple_children_nested_and_self_closing() {
        let xml = "<body xmlns='x'>\n  <message to='a'><body>hi &gt; <b/></body></message>\n  <presence/>\n  <!-- c <x> --><iq a='>'><query/></iq></body>";
        let b = parse_body(xml).unwrap();
        assert_eq!(
            b.children,
            vec!["<message to='a'><body>hi &gt; <b/></body></message>", "<presence/>", "<iq a='>'><query/></iq>"]
        );
    }

    #[test]
    fn parse_cdata_and_empty_bodies() {
        let b = parse_body("<body><m><![CDATA[</m> <x>]]></m></body>").unwrap();
        assert_eq!(b.children, vec!["<m><![CDATA[</m> <x>]]></m>"]);
        assert!(parse_body("<body xmlns='x'/>").unwrap().children.is_empty());
        assert!(parse_body("<body xmlns='x'></body>").unwrap().children.is_empty());
    }

    #[test]
    fn parse_terminate() {
        let b = parse_body("<body type='terminate' condition='item-not-found' xmlns='x'/>").unwrap();
        assert!(b.is_terminate());
        assert_eq!(b.attr("condition"), Some("item-not-found"));
    }

    #[test]
    fn parse_rejects_malformed() {
        for bad in ["", "<html/>", "<body", "<body><m></body>", "<body><m>", "<body a=1/>", "<body><m></n></body>x"] {
            assert_eq!(parse_body(bad), Err(Error::Protocol), "{bad:?}");
        }
    }
}
