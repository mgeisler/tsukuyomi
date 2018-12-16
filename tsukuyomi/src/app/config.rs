pub mod endpoint;
pub mod route;

pub mod prelude {
    #[doc(no_inline)]
    pub use super::{empty, mount, Config, ConfigExt};

    #[doc(no_inline)]
    pub use crate::path;

    pub mod path {
        pub use super::super::route::{catch_all, param, slash};
    }

    pub mod endpoint {
        #[doc(no_inline)]
        pub use super::super::endpoint::{
            allow_only, any, connect, delete, get, head, options, patch, post, put, trace,
        };
    }
}

use {
    super::{
        recognizer::Recognizer,
        scope::{ScopeId, Scopes},
        App, AppInner, Endpoint, EndpointId, ScopeData, Uri,
    },
    crate::{
        core::{Chain, Never},
        handler::{Handler, ModifyHandler},
        output::Responder,
    },
    std::sync::Arc,
};

/// Creates an `App` using the specified configuration.
pub fn configure(prefix: impl AsRef<str>, config: impl Config<()>) -> super::Result<App> {
    let mut recognizer = Recognizer::default();
    let mut scopes = Scopes::new(ScopeData {
        prefix: prefix.as_ref().parse()?,
        default_handler: None,
    });
    config
        .configure(&mut Scope {
            recognizer: &mut recognizer,
            scopes: &mut scopes,
            scope_id: ScopeId::root(),
            modifier: &(),
        })
        .map_err(Into::into)?;

    Ok(App {
        inner: Arc::new(AppInner { recognizer, scopes }),
    })
}

/// A type representing the contextual information in `Scope::configure`.
#[derive(Debug)]
pub struct Scope<'a, M> {
    recognizer: &'a mut Recognizer<Endpoint>,
    scopes: &'a mut Scopes<ScopeData>,
    modifier: &'a M,
    scope_id: ScopeId,
}

impl<'a, M> Scope<'a, M> {
    /// Appends a `Handler` with the specified URI onto the current scope.
    pub fn at<H>(&mut self, uri: Option<&str>, handler: H) -> super::Result<()>
    where
        H: Handler,
        M: ModifyHandler<H>,
        M::Output: Responder,
        <M::Output as Responder>::Future: Send + 'static,
        M::Handler: Send + Sync + 'static,
        <M::Handler as Handler>::Handle: Send + 'static,
    {
        if let Some(uri) = uri {
            let uri: Uri = uri.parse()?;
            let uri = self.scopes[self.scope_id].data.prefix.join(&uri)?;

            let id = EndpointId(self.recognizer.len());
            let scope = &self.scopes[self.scope_id];
            self.recognizer.insert(
                uri.as_str(),
                Endpoint {
                    id,
                    scope: scope.id(),
                    ancestors: scope
                        .ancestors()
                        .into_iter()
                        .cloned()
                        .chain(Some(scope.id()))
                        .collect(),
                    uri: uri.clone(),
                    handler: Box::new(self.modifier.modify(handler)),
                },
            )?;
        } else {
            self.scopes[self.scope_id].data.default_handler =
                Some(Box::new(self.modifier.modify(handler)));
        }
        Ok(())
    }

    /// Creates a sub-scope with the provided prefix onto the current scope.
    pub fn mount(&mut self, prefix: impl AsRef<str>, config: impl Config<M>) -> super::Result<()> {
        let prefix: Uri = prefix.as_ref().parse()?;

        let scope_id = self.scopes.add_node(self.scope_id, {
            let parent = &self.scopes[self.scope_id].data;
            ScopeData {
                prefix: parent.prefix.join(&prefix)?,
                default_handler: None,
            }
        })?;

        config
            .configure(&mut Scope {
                recognizer: &mut *self.recognizer,
                scopes: &mut *self.scopes,
                scope_id,
                modifier: &*self.modifier,
            })
            .map_err(Into::into)?;

        Ok(())
    }

    /// Applies the specified configuration with a `ModifyHandler` on the current scope.
    pub fn modify<M2>(
        &mut self,
        modifier: M2,
        config: impl Config<Chain<&'a M, M2>>,
    ) -> super::Result<()> {
        config
            .configure(&mut Scope {
                recognizer: &mut *self.recognizer,
                scopes: &mut *self.scopes,
                scope_id: self.scope_id,
                modifier: &Chain::new(self.modifier, modifier),
            })
            .map_err(Into::into)
    }
}

/// A trait representing a set of elements that will be registered into a certain scope.
pub trait Config<M> {
    type Error: Into<super::Error>;

    /// Applies this configuration to the specified context.
    fn configure(self, cx: &mut Scope<'_, M>) -> Result<(), Self::Error>;
}

impl<F, M, E> Config<M> for F
where
    F: FnOnce(&mut Scope<'_, M>) -> Result<(), E>,
    E: Into<super::Error>,
{
    type Error = E;

    fn configure(self, cx: &mut Scope<'_, M>) -> Result<(), Self::Error> {
        self(cx)
    }
}

impl<S1, S2, M> Config<M> for Chain<S1, S2>
where
    S1: Config<M>,
    S2: Config<M>,
{
    type Error = super::Error;

    fn configure(self, cx: &mut Scope<'_, M>) -> Result<(), Self::Error> {
        self.left.configure(cx).map_err(Into::into)?;
        self.right.configure(cx).map_err(Into::into)?;
        Ok(())
    }
}

impl<M, S> Config<M> for Option<S>
where
    S: Config<M>,
{
    type Error = S::Error;

    fn configure(self, cx: &mut Scope<'_, M>) -> Result<(), Self::Error> {
        if let Some(scope) = self {
            scope.configure(cx)?;
        }
        Ok(())
    }
}

impl<M, S, E> Config<M> for Result<S, E>
where
    S: Config<M>,
    E: Into<super::Error>,
{
    type Error = super::Error;

    fn configure(self, cx: &mut Scope<'_, M>) -> Result<(), Self::Error> {
        self.map_err(Into::into)?.configure(cx).map_err(Into::into)
    }
}

pub fn empty() -> Empty {
    Empty(())
}

#[derive(Debug)]
pub struct Empty(());

impl<M> Config<M> for Empty {
    type Error = Never;

    fn configure(self, _: &mut Scope<'_, M>) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Creates a `Config` that creates a sub-scope with the provided prefix.
pub fn mount<P>(prefix: P) -> Mount<P, Empty>
where
    P: AsRef<str>,
{
    Mount {
        prefix,
        config: empty(),
    }
}

/// A `Config` that registers a sub-scope with a specific prefix.
#[derive(Debug)]
pub struct Mount<P, T> {
    prefix: P,
    config: T,
}

impl<P, T> Mount<P, T>
where
    P: AsRef<str>,
{
    pub fn with<T2>(self, config: T2) -> Mount<P, Chain<T, T2>> {
        Mount {
            prefix: self.prefix,
            config: Chain::new(self.config, config),
        }
    }
}

impl<P, T, M> Config<M> for Mount<P, T>
where
    P: AsRef<str>,
    T: Config<M>,
{
    type Error = super::Error;

    fn configure(self, scope: &mut Scope<'_, M>) -> Result<(), Self::Error> {
        scope.mount(self.prefix, self.config)
    }
}

/// Crates a `Config` that wraps a config with a `ModifyHandler`.
pub fn modify<M, T>(modifier: M, config: T) -> Modify<M, T> {
    Modify { modifier, config }
}

/// A `Config` that wraps a config with a `ModifyHandler`.
#[derive(Debug)]
pub struct Modify<M, T> {
    modifier: M,
    config: T,
}

impl<M, T, M2> Config<M2> for Modify<M, T>
where
    for<'a> T: Config<Chain<&'a M2, M>>,
{
    type Error = super::Error;

    fn configure(self, cx: &mut Scope<'_, M2>) -> Result<(), Self::Error> {
        cx.modify(self.modifier, self.config)
    }
}

pub trait ConfigExt: Sized {
    fn with<T>(self, config: T) -> Chain<Self, T> {
        Chain::new(self, config)
    }

    /// Creates a `Config` with the specified `ModifyHandler`
    fn modify<M>(self, modifier: M) -> Modify<M, Self> {
        modify(modifier, self)
    }

    fn mount<P, T>(self, prefix: P, config: T) -> Chain<Self, Mount<P, Chain<Empty, T>>>
    where
        P: AsRef<str>,
    {
        self.with(mount(prefix).with(config))
    }
}

impl<T> ConfigExt for T {}