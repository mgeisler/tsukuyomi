//! Components for parsing incoming HTTP requests.

pub mod body;

mod input;
mod request;

#[doc(inline)]
pub use self::body::RequestBody;

#[doc(inline)]
pub use self::input::{Cookies, Input, Params};

#[doc(inline)]
pub use self::request::RequestExt;

pub(crate) use self::input::InputParts;
