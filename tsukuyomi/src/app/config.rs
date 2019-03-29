use {
    super::{
        concurrency::{Concurrency, DefaultConcurrency},
        path::{IntoPath, Path, PathExtractor},
        recognizer::Recognizer,
        scope::{ScopeId, Scopes},
        App, AppInner, ResourceData, RouteData, ScopeData, Uri,
    },
    crate::{
        endpoint::Endpoint,
        handler::ModifyHandler,
        util::{Chain, Never},
    },
    http::Method,
    indexmap::map::{Entry, IndexMap},
    std::{error, fmt, marker::PhantomData, rc::Rc, sync::Arc},
};

/// A type alias of `Result<T, E>` whose error type is restricted to `AppError`.
pub type Result<T> = std::result::Result<T, Error>;

/// An error type which will be thrown from `AppBuilder`.
#[derive(Debug)]
pub struct Error {
    cause: failure::Compat<failure::Error>,
}

impl From<Never> for Error {
    fn from(never: Never) -> Self {
        match never {}
    }
}

impl Error {
    pub fn custom<E>(cause: E) -> Self
    where
        E: Into<failure::Error>,
    {
        Self {
            cause: cause.into().compat(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(f)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        Some(&self.cause)
    }
}

impl<C> App<C>
where
    C: Concurrency,
{
    /// Construct an `App` using the provided function.
    pub fn build<F>(f: F) -> Result<Self>
    where
        F: FnOnce(Scope<'_, (), C>) -> Result<()>,
    {
        let mut app = AppInner {
            recognizer: Recognizer::default(),
            scopes: Scopes::new(ScopeData {
                prefix: Uri::root(),
                default_handler: None,
            }),
        };

        f(Scope {
            app: &mut app,
            scope_id: ScopeId::root(),
            modifier: (),
            _marker: PhantomData,
        })?;

        Ok(Self {
            inner: Arc::new(app),
        })
    }
}

/// A type representing the "scope" in Web application.
#[derive(Debug)]
pub struct Scope<'a, M, C: Concurrency = DefaultConcurrency> {
    app: &'a mut AppInner<C>,
    modifier: M,
    scope_id: ScopeId,
    _marker: PhantomData<Rc<()>>,
}

/// The experimental API for the next version.
impl<'a, M, C> Scope<'a, M, C>
where
    C: Concurrency,
{
    /// Creates a resource onto the current scope and configure with the provided function.
    pub fn at<P>(&mut self, path: P) -> Result<Resource<'_, P::Extractor, &M, C>>
    where
        P: IntoPath,
    {
        let path = path.into_path();
        let uri: Uri = path.uri_str().parse().map_err(Error::custom)?;
        let uri = self.app.scopes[self.scope_id]
            .data
            .prefix
            .join(&uri)
            .map_err(Error::custom)?;

        let scope = &self.app.scopes[self.scope_id];

        let resource = self
            .app
            .recognizer
            .insert(
                uri.as_str(),
                Arc::new(ResourceData {
                    scope: scope.id(),
                    ancestors: scope
                        .ancestors()
                        .iter()
                        .cloned()
                        .chain(Some(scope.id()))
                        .collect(),
                    uri: uri.clone(),
                    routes: vec![],
                    default_route: None,
                    verbs: IndexMap::default(),
                }),
            )
            .map_err(Error::custom)?;

        Ok(Resource {
            resource: Arc::get_mut(resource).expect("the instance has already been shared"),
            modifier: &self.modifier,
            path,
        })
    }

    /// Registers the scope-level fallback handler onto the current scope.
    ///
    /// The fallback handler is called when there are no resources that exactly
    /// matches to the incoming request.
    pub fn fallback<T>(&mut self, endpoint: T) -> Result<()>
    where
        T: Endpoint<()>,
        M: ModifyHandler<EndpointHandler<(), T>>,
        M::Handler: Into<C::Handler>,
    {
        let handler = EndpointHandler::new(endpoint);
        let handler = self.modifier.modify(handler);
        self.app.scopes[self.scope_id].data.default_handler = Some(handler.into());
        Ok(())
    }

    /// Creates a sub-scope onto the current scope.
    pub fn mount<P>(&mut self, prefix: P) -> Result<Scope<'_, &M, C>>
    where
        P: AsRef<str>,
    {
        let prefix: Uri = prefix.as_ref().parse().map_err(Error::custom)?;

        let scope_id = self
            .app
            .scopes
            .add_node(self.scope_id, {
                let parent = &self.app.scopes[self.scope_id].data;
                ScopeData {
                    prefix: parent.prefix.join(&prefix).map_err(Error::custom)?,
                    default_handler: None,
                }
            })
            .map_err(Error::custom)?;

        Ok(Scope {
            app: &mut *self.app,
            scope_id,
            modifier: &self.modifier,
            _marker: PhantomData,
        })
    }

    /// Adds the provided `ModifyHandler` to the stack and executes a configuration.
    ///
    /// Unlike `nest`, this method does not create a scope.
    pub fn with<M2>(&mut self, modifier: M2) -> Scope<'_, Chain<M2, &M>, C> {
        Scope {
            app: &mut *self.app,
            scope_id: self.scope_id,
            modifier: Chain::new(modifier, &self.modifier),
            _marker: PhantomData,
        }
    }

    /// Applies itself to the provided function.
    pub fn done<F, T>(self, f: F) -> T
    where
        F: FnOnce(Self) -> T,
    {
        f(self)
    }
}

/// A set of routes associated with a specific HTTP path.
#[derive(Debug)]
pub struct Resource<'s, P, M, C>
where
    P: PathExtractor,
    C: Concurrency,
{
    resource: &'s mut ResourceData<C>,
    path: Path<P>,
    modifier: M,
}

impl<'s, P, M, C> Resource<'s, P, M, C>
where
    P: PathExtractor,
    C: Concurrency,
{
    /// Creates a `Route` that matches to the specified HTTP methods.
    pub fn route(
        &mut self,
        methods: impl IntoIterator<Item = impl Into<Method>>,
    ) -> Route<'_, P, &M, C> {
        self.route2(Some(methods.into_iter().map(Into::into).collect()))
    }

    fn route2(&mut self, methods: Option<Vec<Method>>) -> Route<'_, P, &M, C> {
        Route {
            resource: &mut *self.resource,
            methods,
            modifier: &self.modifier,
            _marker: PhantomData,
        }
    }

    pub fn get(&mut self) -> Route<'_, P, &M, C> {
        self.route(Some(Method::GET))
    }

    pub fn post(&mut self) -> Route<'_, P, &M, C> {
        self.route(Some(Method::POST))
    }

    pub fn put(&mut self) -> Route<'_, P, &M, C> {
        self.route(Some(Method::PUT))
    }

    pub fn head(&mut self) -> Route<'_, P, &M, C> {
        self.route(Some(Method::HEAD))
    }

    pub fn delete(&mut self) -> Route<'_, P, &M, C> {
        self.route(Some(Method::DELETE))
    }

    pub fn patch(&mut self) -> Route<'_, P, &M, C> {
        self.route(Some(Method::PATCH))
    }

    /// Creates a `Route` that matches to all HTTP methods.
    pub fn any(&mut self) -> Route<'_, P, &M, C> {
        self.route2(None)
    }

    /// Sets an endpoint that matches to all HTTP methods.
    pub fn to<T>(&mut self, endpoint: T) -> Result<()>
    where
        T: Endpoint<P::Output>,
        M: ModifyHandler<EndpointHandler<P, T>>,
        M::Handler: Into<C::Handler>,
    {
        self.any().to(endpoint)
    }

    /// Appends a `ModifyHandler` to the stack applied to the all handlers on this resource.
    pub fn with<M2>(self, modifier: M2) -> Resource<'s, P, Chain<M2, M>, C> {
        Resource {
            resource: self.resource,
            path: self.path,
            modifier: Chain::new(modifier, self.modifier),
        }
    }

    /// Applies itself to the specified function.
    pub fn done<F, T>(self, f: F) -> T
    where
        F: FnOnce(Self) -> T,
    {
        f(self)
    }
}

#[allow(missing_debug_implementations)]
pub struct Route<'a, P, M, C>
where
    P: PathExtractor,
    C: Concurrency,
{
    resource: &'a mut ResourceData<C>,
    methods: Option<Vec<Method>>,
    modifier: M,
    _marker: PhantomData<P>,
}

impl<'a, P, M, C> Route<'a, P, M, C>
where
    P: PathExtractor,
    C: Concurrency,
{
    pub fn with<M2>(self, modifier: M2) -> Route<'a, P, Chain<M2, M>, C> {
        Route {
            resource: self.resource,
            methods: self.methods,
            modifier: Chain::new(modifier, self.modifier),
            _marker: PhantomData,
        }
    }

    pub fn to<T>(self, endpoint: T) -> Result<()>
    where
        T: Endpoint<P::Output>,
        M: ModifyHandler<EndpointHandler<P, T>>,
        M::Handler: Into<C::Handler>,
    {
        let handler = self.modifier.modify(EndpointHandler::new(endpoint));
        let route = RouteData {
            handler: handler.into(),
        };

        if let Some(methods) = self.methods {
            let index = self.resource.routes.len();
            self.resource.routes.push(route);

            for method in methods {
                match self.resource.verbs.entry(method) {
                    Entry::Occupied(..) => {
                        return Err(Error::custom(failure::format_err!("duplicated method")));
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(index);
                    }
                }
            }
        } else {
            if self.resource.default_route.is_some() {
                return Err(Error::custom(failure::format_err!(
                    "the default route handler has already been set"
                )));
            }
            self.resource.default_route = Some(route);
        }
        Ok(())
    }
}

/// A `Handler` that uses on an endpoint tied to a specific HTTP path.
#[allow(missing_debug_implementations)]
pub struct EndpointHandler<P, T> {
    endpoint: Arc<T>,
    _marker: PhantomData<P>,
}

impl<P, T> EndpointHandler<P, T>
where
    P: PathExtractor,
    T: Endpoint<P::Output>,
{
    pub(crate) fn new(endpoint: T) -> Self {
        Self {
            endpoint: Arc::new(endpoint),
            _marker: PhantomData,
        }
    }
}

mod handler {
    use {
        super::{EndpointHandler, PathExtractor},
        crate::{
            endpoint::Endpoint,
            error::Error,
            future::{Poll, TryFuture},
            handler::Handler,
            input::Input,
        },
        std::sync::Arc,
    };

    impl<P, T> Handler for EndpointHandler<P, T>
    where
        P: PathExtractor,
        T: Endpoint<P::Output>,
    {
        type Output = T::Output;
        type Error = Error;
        type Handle = EndpointHandle<P, T>;

        fn handle(&self) -> Self::Handle {
            EndpointHandle {
                state: State::Init(self.endpoint.clone()),
            }
        }
    }

    #[doc(hidden)]
    #[allow(missing_debug_implementations)]
    pub struct EndpointHandle<P, T>
    where
        P: PathExtractor,
        T: Endpoint<P::Output>,
    {
        state: State<P, T>,
    }

    #[allow(missing_debug_implementations)]
    enum State<P, T>
    where
        P: PathExtractor,
        T: Endpoint<P::Output>,
    {
        Init(Arc<T>),
        InFlight(T::Future),
    }

    impl<P, T> TryFuture for EndpointHandle<P, T>
    where
        P: PathExtractor,
        T: Endpoint<P::Output>,
    {
        type Ok = T::Output;
        type Error = Error;

        #[inline]
        fn poll_ready(&mut self, input: &mut Input<'_>) -> Poll<Self::Ok, Self::Error> {
            loop {
                self.state = match self.state {
                    State::Init(ref endpoint) => {
                        let args = P::extract(input.params.as_ref())?;
                        State::InFlight(endpoint.apply(args))
                    }
                    State::InFlight(ref mut in_flight) => {
                        return in_flight.poll_ready(input).map_err(Into::into);
                    }
                };
            }
        }
    }
}
