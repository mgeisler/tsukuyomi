//! Extractors for parsing message body.

use {
    bytes::Bytes,
    mime::Mime,
    serde::de::DeserializeOwned,
    std::{marker::PhantomData, str},
};

#[doc(hidden)]
#[derive(Debug, failure::Fail)]
pub enum ExtractBodyError {
    #[fail(display = "missing the header field `Content-type`")]
    MissingContentType,

    #[fail(
        display = "the header field `Content-type` is not an expected value (expected: {})",
        expected
    )]
    UnexpectedContentType { expected: &'static str },

    #[fail(display = "the header field `Content-type` is not a valid MIME")]
    InvalidMime,

    #[fail(display = "charset in `Content-type` must be equal to `utf-8`")]
    NotUtf8Charset,

    #[fail(display = "the content of message body is invalid: {}", cause)]
    InvalidContent { cause: failure::Error },
}

mod decode {
    use super::*;

    pub trait Decoder<T> {
        fn validate_mime(&self, mime: Option<&Mime>) -> Result<(), ExtractBodyError>;
        fn decode(data: &Bytes) -> Result<T, ExtractBodyError>;
    }

    #[derive(Debug, Default)]
    pub struct PlainTextDecoder(());

    impl<T> Decoder<T> for PlainTextDecoder
    where
        T: DeserializeOwned + 'static,
    {
        fn validate_mime(&self, mime: Option<&Mime>) -> Result<(), ExtractBodyError> {
            if let Some(mime) = mime {
                if mime.type_() != mime::TEXT || mime.subtype() != mime::PLAIN {
                    return Err(ExtractBodyError::UnexpectedContentType {
                        expected: "text/plain",
                    });
                }
                if let Some(charset) = mime.get_param("charset") {
                    if charset != "utf-8" {
                        return Err(ExtractBodyError::NotUtf8Charset);
                    }
                }
            }
            Ok(())
        }

        fn decode(data: &Bytes) -> Result<T, ExtractBodyError> {
            let s = str::from_utf8(&*data).map_err(|cause| ExtractBodyError::InvalidContent {
                cause: cause.into(),
            })?;
            serde_plain::from_str(s).map_err(|cause| ExtractBodyError::InvalidContent {
                cause: cause.into(),
            })
        }
    }

    #[derive(Debug, Default)]
    pub struct JsonDecoder(());

    impl<T> Decoder<T> for JsonDecoder
    where
        T: DeserializeOwned + 'static,
    {
        fn validate_mime(&self, mime: Option<&Mime>) -> Result<(), ExtractBodyError> {
            let mime = mime.ok_or_else(|| ExtractBodyError::MissingContentType)?;
            if *mime != mime::APPLICATION_JSON {
                return Err(ExtractBodyError::UnexpectedContentType {
                    expected: "application/json",
                });
            }
            Ok(())
        }

        fn decode(data: &Bytes) -> Result<T, ExtractBodyError> {
            serde_json::from_slice(&*data).map_err(|cause| ExtractBodyError::InvalidContent {
                cause: cause.into(),
            })
        }
    }

    #[derive(Debug, Default)]
    pub struct UrlencodedDecoder(());

    impl<T> Decoder<T> for UrlencodedDecoder
    where
        T: DeserializeOwned + 'static,
    {
        fn validate_mime(&self, mime: Option<&Mime>) -> Result<(), ExtractBodyError> {
            let mime = mime.ok_or_else(|| ExtractBodyError::MissingContentType)?;
            if *mime != mime::APPLICATION_WWW_FORM_URLENCODED {
                return Err(ExtractBodyError::UnexpectedContentType {
                    expected: "application/x-www-form-urlencoded",
                });
            }
            Ok(())
        }

        fn decode(data: &Bytes) -> Result<T, ExtractBodyError> {
            serde_urlencoded::from_bytes(&*data).map_err(|cause| ExtractBodyError::InvalidContent {
                cause: cause.into(),
            })
        }
    }
}

fn decoded<T, D>(decoder: D) -> self::decoded::Decoded<T, D>
where
    T: Send + 'static,
    D: self::decode::Decoder<T>,
{
    self::decoded::Decoded {
        decoder,
        _marker: PhantomData,
    }
}

mod decoded {
    use {
        crate::{
            error::Error,
            extractor::Extractor,
            input::{body::RequestBody, header::ContentType, Input},
        },
        futures01::{
            future::{err, Either, FutureResult},
            Future,
        },
        std::marker::PhantomData,
    };

    #[derive(Debug)]
    pub struct Decoded<T, D> {
        pub(super) decoder: D,
        pub(super) _marker: PhantomData<fn() -> T>,
    }

    impl<T, D> Extractor for Decoded<T, D>
    where
        T: Send + 'static,
        D: super::decode::Decoder<T> + 'static,
    {
        type Output = (T,);
        type Error = Error;
        type Future = Either<FutureResult<(T,), Error>, DecodedFuture<T, D>>;

        fn extract(&self, input: &mut Input<'_>) -> Self::Future {
            if let Err(e) = {
                crate::input::header::parse::<ContentType>(input) //
                    .and_then(|mime_opt| {
                        self.decoder
                            .validate_mime(mime_opt)
                            .map_err(crate::error::bad_request)
                    })
            } {
                return Either::A(err(e));
            }

            let read_all = match input.locals.remove(&RequestBody::KEY) {
                Some(body) => body.read_all(),
                None => return Either::A(err(super::stolen_payload())),
            };

            Either::B(DecodedFuture {
                read_all,
                _marker: PhantomData,
            })
        }
    }

    #[allow(missing_debug_implementations)]
    pub struct DecodedFuture<T, D> {
        read_all: crate::input::body::ReadAll,
        _marker: PhantomData<fn(D) -> T>,
    }

    impl<T, D> Future for DecodedFuture<T, D>
    where
        D: super::decode::Decoder<T>,
    {
        type Item = (T,);
        type Error = Error;

        fn poll(&mut self) -> futures01::Poll<Self::Item, Self::Error> {
            let data = futures01::try_ready!(self.read_all.poll());
            D::decode(&data)
                .map(|out| (out,).into())
                .map_err(crate::error::bad_request)
        }
    }
}

#[inline]
pub fn plain<T>() -> self::decoded::Decoded<T, impl self::decode::Decoder<T>>
where
    T: DeserializeOwned + Send + 'static,
{
    self::decoded(self::decode::PlainTextDecoder::default())
}

#[inline]
pub fn json<T>() -> self::decoded::Decoded<T, impl self::decode::Decoder<T>>
where
    T: DeserializeOwned + Send + 'static,
{
    self::decoded(self::decode::JsonDecoder::default())
}

#[inline]
pub fn urlencoded<T>() -> self::decoded::Decoded<T, impl self::decode::Decoder<T>>
where
    T: DeserializeOwned + Send + 'static,
{
    self::decoded(self::decode::UrlencodedDecoder::default())
}

pub fn read_all() -> self::read_all::ReadAll {
    self::read_all::ReadAll(())
}

mod read_all {
    use {
        crate::{
            error::Error,
            extractor::Extractor,
            input::{body::RequestBody, Input},
        },
        bytes::Bytes,
        futures01::{
            future::{self, err, Either, FutureResult},
            Future,
        },
    };

    #[derive(Debug)]
    pub struct ReadAll(pub(super) ());

    impl Extractor for ReadAll {
        type Output = (Bytes,);
        type Error = Error;
        type Future = Either<
            FutureResult<(Bytes,), Error>,
            future::MapErr<
                future::Map<crate::input::body::ReadAll, fn(Bytes) -> (Bytes,)>,
                fn(hyper::Error) -> Error,
            >,
        >;

        fn extract(&self, input: &mut Input<'_>) -> Self::Future {
            match input.locals.remove(&RequestBody::KEY) {
                Some(body) => Either::B(
                    body.read_all()
                        .map((|x| (x,)) as fn(_) -> _)
                        .map_err(Into::into as fn(_) -> _),
                ),
                None => Either::A(err(super::stolen_payload())),
            }
        }
    }
}

pub fn stream() -> self::stream::Stream {
    self::stream::Stream(())
}

mod stream {
    use crate::{
        error::Error,
        extractor::Extractor,
        input::{body::RequestBody, Input},
    };

    #[derive(Debug)]
    pub struct Stream(pub(super) ());

    impl Extractor for Stream {
        type Output = (RequestBody,);
        type Error = Error;
        type Future = futures01::future::FutureResult<Self::Output, Self::Error>;

        fn extract(&self, input: &mut Input<'_>) -> Self::Future {
            match input.locals.remove(&RequestBody::KEY) {
                Some(body) => futures01::future::ok((body,)),
                None => futures01::future::err(super::stolen_payload()),
            }
        }
    }
}

fn stolen_payload() -> crate::error::Error {
    crate::error::internal_server_error("The instance of raw RequestBody has already stolen.")
}
