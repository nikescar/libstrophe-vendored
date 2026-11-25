//! Certificate parsing and validation
//!
//! Extracts XMPP addresses (id-on-xmppAddr) from X.509 certificates

use x509_parser::prelude::*;
use x509_parser::der_parser::asn1_rs::Any;
use std::ffi::CString;
use std::os::raw::c_char;

/// id-on-xmppAddr OID from RFC 6120
const ID_ON_XMPPADDR_OID: &[u64] = &[1, 3, 6, 1, 5, 5, 7, 8, 5];

/// Extract XMPP addresses from certificate SANs
///
/// Returns list of UTF-8 XMPP addresses (JIDs) found in id-on-xmppAddr otherName entries
pub fn extract_xmpp_addrs(cert_der: &[u8]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let (_, cert) = X509Certificate::from_der(cert_der)?;
    let mut xmpp_addrs = Vec::new();

    // Check for SAN extension
    if let Some(san_ext) = cert.subject_alternative_name()? {
        for san in &san_ext.value.general_names {
            if let GeneralName::OtherName(oid, value) = san {
                // Check if this is id-on-xmppAddr
                if let Some(oid_iter) = oid.iter() {
                    let oid_arcs: Vec<u64> = oid_iter.collect();
                    if oid_arcs == ID_ON_XMPPADDR_OID {
                        // Parse the UTF8String inside the otherName
                        if let Ok((_, any)) = Any::from_der(value) {
                            if let Ok(utf8_str) = any.as_str() {
                                xmpp_addrs.push(utf8_str.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(xmpp_addrs)
}

/// Convert Vec<String> to Vec<CString> for C FFI
pub fn strings_to_c(strings: Vec<String>) -> Vec<CString> {
    strings
        .into_iter()
        .filter_map(|s| CString::new(s).ok())
        .collect()
}

/// Leak CString as *mut c_char for C ownership
pub fn leak_cstring(s: CString) -> *mut c_char {
    s.into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_xmpp_addrs_invalid() {
        // Invalid cert should return error
        let invalid_cert = b"not a certificate";
        assert!(extract_xmpp_addrs(invalid_cert).is_err());
    }

    #[test]
    fn test_strings_to_c() {
        let strings = vec!["test@example.com".to_string(), "user@domain.org".to_string()];
        let cstrings = strings_to_c(strings);
        assert_eq!(cstrings.len(), 2);
    }
}
