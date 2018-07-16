//! The definition of components for serving an HTTP application by using `App`.

use futures::future::lazy;
use futures::{self, Async, Future, Poll};
use http::header::HeaderValue;
use http::{header, Request, Response, StatusCode};
use hyper::body::Body;
use hyper::service::{NewService, Service};
use std::mem;
use tokio;

use error::{CritError, Error};
use handler::Handle;
use input::{Input, InputParts, RequestBody};
use modifier::{AfterHandle, BeforeHandle};
use output::upgrade::UpgradeContext;
use output::{Output, ResponseBody};

use super::router::RecognizeErrorKind;
use super::App;

impl App {
    /// Creates a new `AppService` to manage a session.
    pub fn new_service(&self) -> AppService {
        AppService { app: self.clone() }
    }
}

impl NewService for App {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = CritError;
    type Service = AppService;
    type InitError = CritError;
    type Future = futures::future::FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        futures::future::ok(self.new_service())
    }
}

/// A `Service` representation of the application, created by `App`.
#[derive(Debug)]
pub struct AppService {
    app: App,
}

impl AppService {
    #[allow(missing_docs)]
    pub fn dispatch_request(&mut self, request: Request<RequestBody>) -> AppServiceFuture {
        AppServiceFuture {
            request: Some(request),
            parts: None,
            app: self.app.clone(),
            pipeline: Pipeline::Start,
        }
    }
}

impl Service for AppService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = CritError;
    type Future = AppServiceFuture;

    #[inline]
    fn call(&mut self, request: Request<Self::ReqBody>) -> Self::Future {
        self.dispatch_request(request.map(RequestBody::from_hyp))
    }
}

/// A future for managing an incoming HTTP request, created by `AppService`.
#[must_use = "futures do nothing unless polled"]
#[derive(Debug)]
pub struct AppServiceFuture {
    pipeline: Pipeline,
    request: Option<Request<RequestBody>>,
    parts: Option<InputParts>,
    app: App,
}

#[derive(Debug)]
enum Pipeline {
    Start,
    Recognized,
    BeforeHandle { in_flight: BeforeHandle, current: usize },
    Handle(Handle),
    AfterHandle { in_flight: AfterHandle, current: usize },
    Done,
}

#[derive(Debug)]
enum PipelineErrorKind {
    Recognize(RecognizeErrorKind),
    Http(Error),
}

impl From<Error> for PipelineErrorKind {
    fn from(err: Error) -> Self {
        PipelineErrorKind::Http(err)
    }
}

impl AppServiceFuture {
    fn poll_pipeline(&mut self) -> Poll<Output, PipelineErrorKind> {
        use self::Pipeline::*;

        macro_rules! input {
            () => {
                Input {
                    request: self.request
                        .as_mut()
                        .expect("This future has already polled"),
                    parts: self.parts.as_mut().expect("This future has already polled"),
                    app: &self.app,
                }
            };
        }

        loop {
            let output = match self.pipeline {
                Start | Recognized => None,
                BeforeHandle { ref mut in_flight, .. } => {
                    try_ready!(in_flight.poll_ready(&mut input!()));
                    None
                }
                Handle(ref mut in_flight) => Some(try_ready!(in_flight.poll_ready(&mut input!()))),
                AfterHandle { ref mut in_flight, .. } => Some(try_ready!(in_flight.poll_ready(&mut input!()))),
                Done => panic!("unexpected state"),
            };

            self.pipeline = match (mem::replace(&mut self.pipeline, Done), output) {
                (Start, None) => {
                    let request = self.request.as_ref().expect("This future has already polled");
                    let recognize = self.app
                        .router()
                        .recognize(request.uri().path(), request.method())
                        .map_err(PipelineErrorKind::Recognize)?;
                    self.parts = Some(InputParts::new(recognize));
                    Recognized
                }

                (Recognized, None) => match self.app.modifiers().get(0) {
                    Some(modifier) => BeforeHandle {
                        in_flight: modifier.before_handle(&mut input!()),
                        current: 1,
                    },
                    None => {
                        let mut input = input!();
                        let endpoint = self.app.endpoint(input.parts.recognize.endpoint_id).expect("");
                        Handle(endpoint.handler().handle(&mut input))
                    }
                },

                (BeforeHandle { current, .. }, None) => match self.app.modifiers().get(current) {
                    Some(modifier) => BeforeHandle {
                        in_flight: modifier.before_handle(&mut input!()),
                        current: current + 1,
                    },
                    None => {
                        let mut input = input!();
                        let endpoint = self.app.endpoint(input.parts.recognize.endpoint_id).expect("");
                        Handle(endpoint.handler().handle(&mut input))
                    }
                },

                (Handle(..), Some(output)) => {
                    if self.app.modifiers().is_empty() {
                        break Ok(Async::Ready(output));
                    }
                    let current = self.app.modifiers().len() - 1;
                    let modifier = &self.app.modifiers()[current];
                    AfterHandle {
                        in_flight: modifier.after_handle(&mut input!(), output),
                        current: current,
                    }
                }

                (AfterHandle { current, .. }, Some(output)) => {
                    if current == 0 {
                        break Ok(Async::Ready(output));
                    }
                    let current = current - 1;
                    let modifier = &self.app.modifiers()[current];
                    AfterHandle {
                        in_flight: modifier.after_handle(&mut input!(), output),
                        current: current,
                    }
                }

                _ => panic!("unexpected state"),
            }
        }
    }

    #[allow(missing_docs)]
    pub fn poll_ready(&mut self) -> Poll<Response<ResponseBody>, CritError> {
        match self.poll_pipeline() {
            Ok(Async::Ready(output)) => self.handle_response(output).map(Async::Ready),
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Err(err) => {
                self.pipeline = Pipeline::Done;
                self.handle_error(err).map(Async::Ready)
            }
        }
    }

    fn handle_response(&mut self, output: Output) -> Result<Response<ResponseBody>, CritError> {
        let (mut response, handler) = output.deconstruct();

        let parts = self.parts.take().expect("This future has already polled");
        let InputParts { cookies, .. } = parts;

        cookies.append_to(response.headers_mut());

        // append the value of Content-Length to the response header if missing.
        if let Some(len) = response.body().content_length() {
            response
                .headers_mut()
                .entry(header::CONTENT_LENGTH)?
                .or_insert_with(|| {
                    // safety: '0'-'9' is ascci.
                    // TODO: more efficient
                    unsafe { HeaderValue::from_shared_unchecked(len.to_string().into()) }
                });
        }

        if let Some(handler) = handler {
            debug_assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

            let mut request = self.request.take().expect("This future has already polled.");
            let on_upgrade = request
                .body_mut()
                .on_upgrade()
                .ok_or_else(|| format_err!("The request body has already gone").compat())?;
            let request = request.map(mem::drop);

            tokio::spawn(lazy(move || {
                on_upgrade.map_err(|_| error!("")).and_then(|upgraded| {
                    let cx = UpgradeContext {
                        io: upgraded,
                        request: request,
                        _priv: (),
                    };
                    handler.upgrade(cx)
                })
            }));
        }

        Ok(response)
    }

    fn handle_error(&mut self, err: PipelineErrorKind) -> Result<Response<ResponseBody>, CritError> {
        match err {
            PipelineErrorKind::Recognize(RecognizeErrorKind::NotFound) => self.handle_http_error(Error::not_found()),
            PipelineErrorKind::Recognize(RecognizeErrorKind::MethodNotAllowed) => {
                self.handle_http_error(Error::method_not_allowed())
            }
            PipelineErrorKind::Recognize(RecognizeErrorKind::FallbackOptions { entry_id: i, .. }) => {
                let entry = self.app.router().entry(i).expect("invalid entry ID");
                let response = entry.fallback_options_response();
                Ok(response.map(Into::into))
            }
            PipelineErrorKind::Http(err) => self.handle_http_error(err),
        }
    }

    fn handle_http_error(&mut self, err: Error) -> Result<Response<ResponseBody>, CritError> {
        if let Some(err) = err.as_http_error() {
            let request = self.request
                .take()
                .expect("This future has already polled")
                .map(mem::drop);
            let response = self.app.error_handler().handle_error(err, &request)?;
            return Ok(response);
        }
        Err(err.into_critical()
            .expect("unexpected condition in AppServiceFuture::handle_error"))
    }
}

impl futures::Future for AppServiceFuture {
    type Item = Response<Body>;
    type Error = CritError;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        self.poll_ready()
            .map(|x| x.map(|response| response.map(ResponseBody::into_hyp)))
    }
}
