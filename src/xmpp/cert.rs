use super::error::borrow_str;
use super::opaque;
use crate::ffi;
use std::ops::Deref;
use std::ptr::NonNull;

pub use ffi::xmpp_cert_element_t as CertElement;

/// Every certificate element, in order (excludes `XMPP_CERT_ELEMENT_MAX`).
pub const CERT_ELEMENTS: [CertElement; 10] = {
    use CertElement::*;
    [
        XMPP_CERT_VERSION,
        XMPP_CERT_SERIALNUMBER,
        XMPP_CERT_SUBJECT,
        XMPP_CERT_ISSUER,
        XMPP_CERT_NOTBEFORE,
        XMPP_CERT_NOTAFTER,
        XMPP_CERT_KEYALG,
        XMPP_CERT_SIGALG,
        XMPP_CERT_FINGERPRINT_SHA1,
        XMPP_CERT_FINGERPRINT_SHA256,
    ]
};

opaque!(
    /// Borrowed TLS certificate (`xmpp_tlscert_t`).
    TlsCertRef,
    ffi::xmpp_tlscert_t
);

impl TlsCertRef {
    pub fn pem(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_tlscert_get_pem(self.as_ptr())) }
    }

    pub fn dnsname(&self, n: usize) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_tlscert_get_dnsname(self.as_ptr(), n)) }
    }

    /// All subjectAltName dNSName entries.
    pub fn dnsnames(&self) -> impl Iterator<Item = &str> {
        (0..).map_while(move |n| self.dnsname(n))
    }

    pub fn get(&self, element: CertElement) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_tlscert_get_string(self.as_ptr(), element)) }
    }

    /// Human readable name of `element`.
    pub fn description(element: CertElement) -> Option<&'static str> {
        unsafe { borrow_str(ffi::xmpp_tlscert_get_description(element)) }
    }
}

/// Owned certificate (e.g. from [`super::Conn::peer_cert`]); freed on drop.
pub struct TlsCert<'a> {
    raw: NonNull<ffi::xmpp_tlscert_t>,
    _conn: std::marker::PhantomData<&'a ()>,
}

impl TlsCert<'_> {
    /// # Safety
    /// `p` must be null or an owned certificate valid for `'a`.
    pub(crate) unsafe fn from_raw<'a>(p: *mut ffi::xmpp_tlscert_t) -> Option<TlsCert<'a>> {
        NonNull::new(p).map(|raw| TlsCert { raw, _conn: std::marker::PhantomData })
    }
}

impl Deref for TlsCert<'_> {
    type Target = TlsCertRef;
    fn deref(&self) -> &TlsCertRef {
        unsafe { TlsCertRef::from_ptr(self.raw.as_ptr()) }
    }
}

impl Drop for TlsCert<'_> {
    fn drop(&mut self) {
        unsafe { ffi::xmpp_tlscert_free(self.raw.as_ptr()) }
    }
}
