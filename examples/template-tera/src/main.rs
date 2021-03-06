use {
    crate::support_tera::{Template, WithTera},
    exitfailure::ExitFailure,
    serde::Serialize,
    tsukuyomi::{endpoint::builder as endpoint, path, server::Server, App},
};

#[derive(Debug, Serialize)]
struct Index {
    name: String,
}

impl Template for Index {
    fn template_name(&self) -> &str {
        "index.html"
    }
}

fn main() -> Result<(), ExitFailure> {
    let engine = tera::compile_templates!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/**/*"));

    let app = App::build(|s| {
        s.at(path!("/:name"), WithTera::from(engine), {
            endpoint::call(|name| Index { name }) //
        })
    })?;

    let mut server = Server::new(app)?;
    server.bind("127.0.0.1:4000")?;
    server.run_forever();

    Ok(())
}

mod support_tera {
    use {
        http::{header::HeaderValue, Response},
        std::sync::Arc,
        tera::Tera,
        tsukuyomi::{
            error::Error,
            future::{Poll, TryFuture},
            handler::{metadata::Metadata, Handler, ModifyHandler},
            input::Input,
        },
    };

    pub trait Template: serde::Serialize {
        fn template_name(&self) -> &str;
        fn extension(&self) -> Option<&str> {
            None
        }
    }

    #[derive(Debug)]
    pub struct WithTera(Arc<Tera>);

    impl From<Tera> for WithTera {
        fn from(engine: Tera) -> Self {
            WithTera(Arc::new(engine))
        }
    }

    impl<H> ModifyHandler<H> for WithTera
    where
        H: Handler,
        H::Output: Template,
    {
        type Output = Response<String>;
        type Error = Error;
        type Handler = WithTeraHandler<H>;

        fn modify(&self, inner: H) -> Self::Handler {
            WithTeraHandler {
                inner,
                engine: self.0.clone(),
            }
        }
    }

    #[derive(Debug)]
    pub struct WithTeraHandler<H> {
        inner: H,
        engine: Arc<Tera>,
    }

    impl<H> Handler for WithTeraHandler<H>
    where
        H: Handler,
        H::Output: Template,
    {
        type Output = Response<String>;
        type Error = Error;
        type Handle = WithTeraHandle<H::Handle>;

        fn metadata(&self) -> Metadata {
            self.inner.metadata()
        }

        fn handle(&self) -> Self::Handle {
            WithTeraHandle {
                inner: self.inner.handle(),
                engine: self.engine.clone(),
            }
        }
    }

    #[derive(Debug)]
    pub struct WithTeraHandle<H> {
        inner: H,
        engine: Arc<Tera>,
    }

    impl<H> TryFuture for WithTeraHandle<H>
    where
        H: TryFuture,
        H::Ok: Template,
    {
        type Ok = Response<String>;
        type Error = Error;

        fn poll_ready(&mut self, input: &mut Input<'_>) -> Poll<Self::Ok, Self::Error> {
            let ctx = futures::try_ready!(self.inner.poll_ready(input).map_err(Into::into));
            let content_type = HeaderValue::from_static(
                ctx.extension()
                    .and_then(mime_guess::get_mime_type_str)
                    .unwrap_or("text/html; charset=utf-8"),
            );
            self.engine
                .render(ctx.template_name(), &ctx)
                .map(|body| {
                    Response::builder()
                        .header("content-type", content_type)
                        .body(body)
                        .expect("should be a valid response")
                        .into()
                })
                .map_err(|e| tsukuyomi::error::internal_server_error(e.to_string()))
        }
    }
}
