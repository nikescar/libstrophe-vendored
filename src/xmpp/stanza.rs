use super::ctx::ContextRef;
use super::error::{borrow_str, check, cstr, opt_cstr, opt_ptr, take_string, Error, Result};
use super::opaque;
use crate::ffi;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;

opaque!(
    /// Borrowed stanza (`xmpp_stanza_t`): read-only view, e.g. an incoming
    /// stanza in a handler or a child of an owned [`Stanza`].
    StanzaRef,
    ffi::xmpp_stanza_t
);

impl StanzaRef {
    fn p(&self) -> *mut ffi::xmpp_stanza_t {
        self.as_ptr()
    }

    /// Context the stanza was allocated from.
    pub fn context(&self) -> &ContextRef {
        unsafe { ContextRef::from_ptr(ffi::xmpp_stanza_get_context(self.p())) }
    }

    pub fn is_text(&self) -> bool {
        unsafe { ffi::xmpp_stanza_is_text(self.p()) != 0 }
    }

    pub fn is_tag(&self) -> bool {
        unsafe { ffi::xmpp_stanza_is_tag(self.p()) != 0 }
    }

    pub fn name(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_stanza_get_name(self.p())) }
    }

    pub fn ns(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_stanza_get_ns(self.p())) }
    }

    /// The `type` attribute.
    pub fn stanza_type(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_stanza_get_type(self.p())) }
    }

    pub fn id(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_stanza_get_id(self.p())) }
    }

    pub fn to(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_stanza_get_to(self.p())) }
    }

    pub fn from(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_stanza_get_from(self.p())) }
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        let n = cstr(name).ok()?;
        unsafe { borrow_str(ffi::xmpp_stanza_get_attribute(self.p(), n.as_ptr())) }
    }

    /// Concatenated text content (of this text node or of a tag's text children).
    pub fn text(&self) -> Option<String> {
        unsafe { take_string(self.context().as_ptr(), ffi::xmpp_stanza_get_text(self.p())) }
    }

    /// Text of the `<body/>` child of a message.
    pub fn body(&self) -> Option<String> {
        unsafe { take_string(self.context().as_ptr(), ffi::xmpp_message_get_body(self.p())) }
    }

    /// Serialize to XML.
    pub fn to_text(&self) -> Result<String> {
        let mut buf = std::ptr::null_mut();
        let mut len = 0usize;
        unsafe {
            check(ffi::xmpp_stanza_to_text(self.p(), &mut buf, &mut len))?;
            take_string(self.context().as_ptr(), buf).ok_or(Error::Null)
        }
    }

    pub fn children(&self) -> Children<'_> {
        Children { next: unsafe { StanzaRef::from_ptr_opt(ffi::xmpp_stanza_get_children(self.p())) } }
    }

    /// Next sibling.
    pub fn next(&self) -> Option<&StanzaRef> {
        unsafe { StanzaRef::from_ptr_opt(ffi::xmpp_stanza_get_next(self.p())) }
    }

    pub fn child_by_name(&self, name: &str) -> Option<&StanzaRef> {
        let n = cstr(name).ok()?;
        unsafe { StanzaRef::from_ptr_opt(ffi::xmpp_stanza_get_child_by_name(self.p(), n.as_ptr())) }
    }

    pub fn child_by_ns(&self, ns: &str) -> Option<&StanzaRef> {
        let n = cstr(ns).ok()?;
        unsafe { StanzaRef::from_ptr_opt(ffi::xmpp_stanza_get_child_by_ns(self.p(), n.as_ptr())) }
    }

    pub fn child_by_name_and_ns(&self, name: &str, ns: &str) -> Option<&StanzaRef> {
        let (n, s) = (cstr(name).ok()?, cstr(ns).ok()?);
        unsafe {
            StanzaRef::from_ptr_opt(ffi::xmpp_stanza_get_child_by_name_and_ns(self.p(), n.as_ptr(), s.as_ptr()))
        }
    }

    /// New stanza addressed back to the sender (same name, id; `to` = `from`).
    pub fn reply(&self) -> Result<Stanza<'_>> {
        unsafe { Stanza::from_raw(ffi::xmpp_stanza_reply(self.p())) }
    }

    /// Error reply with `<error type=..><condition/><text>..</text></error>`.
    pub fn reply_error(&self, error_type: &str, condition: &str, text: Option<&str>) -> Result<Stanza<'_>> {
        let (t, c, x) = (cstr(error_type)?, cstr(condition)?, opt_cstr(text)?);
        unsafe { Stanza::from_raw(ffi::xmpp_stanza_reply_error(self.p(), t.as_ptr(), c.as_ptr(), opt_ptr(&x))) }
    }

    /// Deep copy.
    pub fn copy(&self) -> Result<Stanza<'_>> {
        unsafe { Stanza::from_raw(ffi::xmpp_stanza_copy(self.p())) }
    }
}

/// Iterator over a stanza's children.
pub struct Children<'a> {
    next: Option<&'a StanzaRef>,
}

impl<'a> Iterator for Children<'a> {
    type Item = &'a StanzaRef;
    fn next(&mut self) -> Option<&'a StanzaRef> {
        let cur = self.next?;
        self.next = cur.next();
        Some(cur)
    }
}

/// Owned stanza reference; released on drop. Setters need `&mut self`, so
/// borrowed children can't be observed while the tree is mutated.
pub struct Stanza<'cx> {
    raw: NonNull<ffi::xmpp_stanza_t>,
    _ctx: PhantomData<&'cx ContextRef>,
}

impl<'cx> Stanza<'cx> {
    /// # Safety
    /// `p` must be null or an owned reference to a stanza whose context
    /// outlives `'cx`.
    unsafe fn from_raw(p: *mut ffi::xmpp_stanza_t) -> Result<Self> {
        NonNull::new(p).map(|raw| Stanza { raw, _ctx: PhantomData }).ok_or(Error::Null)
    }

    /// Empty stanza; give it a name (tag) or text.
    pub fn new(ctx: &'cx ContextRef) -> Self {
        unsafe { Self::from_raw(ffi::xmpp_stanza_new(ctx.as_ptr())) }.expect("xmpp_stanza_new: out of memory")
    }

    /// Parse one XML element.
    pub fn from_str(ctx: &'cx ContextRef, xml: &str) -> Result<Self> {
        let x = cstr(xml)?;
        unsafe { Self::from_raw(ffi::xmpp_stanza_new_from_string(ctx.as_ptr(), x.as_ptr())) }
    }

    /// `<iq type=.. id=..>`
    pub fn iq(ctx: &'cx ContextRef, type_: &str, id: &str) -> Result<Self> {
        let (t, i) = (cstr(type_)?, cstr(id)?);
        unsafe { Self::from_raw(ffi::xmpp_iq_new(ctx.as_ptr(), t.as_ptr(), i.as_ptr())) }
    }

    /// `<message type=.. to=.. id=..>`
    pub fn message(ctx: &'cx ContextRef, type_: Option<&str>, to: &str, id: Option<&str>) -> Result<Self> {
        let (t, to, i) = (opt_cstr(type_)?, cstr(to)?, opt_cstr(id)?);
        unsafe { Self::from_raw(ffi::xmpp_message_new(ctx.as_ptr(), opt_ptr(&t), to.as_ptr(), opt_ptr(&i))) }
    }

    /// `<presence/>`
    pub fn presence(ctx: &'cx ContextRef) -> Result<Self> {
        unsafe { Self::from_raw(ffi::xmpp_presence_new(ctx.as_ptr())) }
    }

    fn p(&self) -> *mut ffi::xmpp_stanza_t {
        self.raw.as_ptr()
    }

    fn set(&mut self, f: unsafe extern "C" fn(*mut ffi::xmpp_stanza_t, *const std::ffi::c_char) -> i32, v: &str) -> Result<()> {
        let v = cstr(v)?;
        check(unsafe { f(self.p(), v.as_ptr()) })
    }

    pub fn set_name(&mut self, name: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_set_name, name)
    }

    pub fn set_ns(&mut self, ns: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_set_ns, ns)
    }

    pub fn set_type(&mut self, type_: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_set_type, type_)
    }

    pub fn set_id(&mut self, id: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_set_id, id)
    }

    pub fn set_to(&mut self, to: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_set_to, to)
    }

    pub fn set_from(&mut self, from: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_set_from, from)
    }

    /// Make this a text node with content `text`.
    pub fn set_text(&mut self, text: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_set_text, text)
    }

    /// Replace/create the `<body/>` of a message.
    pub fn set_body(&mut self, body: &str) -> Result<()> {
        self.set(ffi::xmpp_message_set_body, body)
    }

    pub fn set_attribute(&mut self, key: &str, value: &str) -> Result<()> {
        let (k, v) = (cstr(key)?, cstr(value)?);
        check(unsafe { ffi::xmpp_stanza_set_attribute(self.p(), k.as_ptr(), v.as_ptr()) })
    }

    pub fn del_attribute(&mut self, key: &str) -> Result<()> {
        self.set(ffi::xmpp_stanza_del_attribute, key)
    }

    /// Append `child` (ownership moves into this stanza). Both must come from
    /// the same context, else [`Error::InvalidOp`].
    pub fn add_child(&mut self, child: Stanza<'_>) -> Result<()> {
        if child.context().as_ptr() != self.context().as_ptr() {
            return Err(Error::InvalidOp);
        }
        // do_clone=0 hands our reference to the parent; C never fails here.
        check(unsafe { ffi::xmpp_stanza_add_child_ex(self.p(), child.p(), 0) })?;
        std::mem::forget(child);
        Ok(())
    }
}

impl Deref for Stanza<'_> {
    type Target = StanzaRef;
    fn deref(&self) -> &StanzaRef {
        unsafe { StanzaRef::from_ptr(self.p()) }
    }
}

impl Drop for Stanza<'_> {
    fn drop(&mut self) {
        unsafe { ffi::xmpp_stanza_release(self.p()) };
    }
}

impl std::fmt::Debug for StanzaRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.to_text() {
            Ok(s) => f.write_str(&s),
            Err(e) => write!(f, "<stanza: {e}>"),
        }
    }
}
