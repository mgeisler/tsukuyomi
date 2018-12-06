//! Extractors for accessing HTTP header fields.

use {
    super::Extractor,
    crate::{common::Never, error::Error},
    http::header::{HeaderMap, HeaderName, HeaderValue},
    mime::Mime,
};

pub trait FromHeaderValue: Sized + 'static {
    type Error: Into<Error>;

    fn from_header_value(h: &HeaderValue) -> Result<Self, Self::Error>;
}

impl FromHeaderValue for String {
    type Error = Error;

    #[inline]
    fn from_header_value(h: &HeaderValue) -> Result<Self, Self::Error> {
        Self::from_utf8(h.as_bytes().to_vec()).map_err(crate::error::bad_request)
    }
}

pub fn header<T>(name: HeaderName) -> impl Extractor<Output = (T,)>
where
    T: FromHeaderValue + Send,
{
    super::ready(move |input| match input.request.headers().get(&name) {
        Some(h) => T::from_header_value(h).map_err(Into::into),
        None => Err(crate::error::bad_request(format!(
            "missing header field: {}",
            name
        ))),
    })
}

pub fn exact<T>(name: HeaderName, value: T) -> impl Extractor<Output = ()>
where
    T: PartialEq<HeaderValue> + Send + Sync + 'static,
{
    super::guard(move |input| match input.request.headers().get(&name) {
        Some(h) if value.eq(h) => Ok(()),
        Some(..) => Err(crate::error::bad_request(format!(
            "mismatched header field: {}",
            name
        ))),
        None => Err(crate::error::bad_request(format!(
            "missing header field: {}",
            name
        ))),
    })
}

/// Creates an extractor which parses the header field `Content-type`.
pub fn content_type() -> impl Extractor<Output = (Mime,)> {
    super::ready(|input| {
        input
            .content_type()?
            .cloned()
            .ok_or_else(|| crate::error::bad_request("missing Content-type"))
    })
}

pub fn clone_headers() -> impl Extractor<Output = (HeaderMap,)> {
    super::ready(|input| Ok::<_, Never>(input.request.headers().clone()))
}
