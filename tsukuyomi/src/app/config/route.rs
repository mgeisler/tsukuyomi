use {
    super::{
        super::uri::{Uri, UriComponent},
        Config, Scope,
    },
    crate::{
        core::{Chain, TryInto},
        endpoint::Endpoint,
        extractor::Extractor,
        generic::{Combine, Tuple},
        handler::{Handler, ModifyHandler},
        input::{
            param::{FromPercentEncoded, PercentEncoded},
            Input,
        },
        output::Responder,
    },
    std::{marker::PhantomData, sync::Arc},
};

#[derive(Debug)]
pub struct Route<H> {
    uri: Option<Uri>,
    handler: H,
}

impl<H> Route<H>
where
    H: Handler,
{
    pub fn new(handler: H) -> Self {
        Self { uri: None, handler }
    }

    pub fn uri(self, uri: impl TryInto<Uri>) -> crate::app::Result<Self> {
        Ok(Self {
            uri: Some(uri.try_into()?),
            ..self
        })
    }
}

impl<H, M> Config<M> for Route<H>
where
    H: Handler,
    M: ModifyHandler<H>,
    M::Output: Responder,
    M::Handler: Send + Sync + 'static,
{
    type Error = crate::app::Error;

    fn configure(self, scope: &mut Scope<'_, M>) -> Result<(), Self::Error> {
        scope.at(self.uri.as_ref().map(|uri| uri.as_str()), self.handler)
    }
}

mod tags {
    #[derive(Debug)]
    pub struct Completed(());

    #[derive(Debug)]
    pub struct Incomplete(());
}

#[derive(Debug)]
pub struct Context<'a> {
    components: Vec<UriComponent>,
    _marker: PhantomData<&'a mut ()>,
}

pub trait PathConfig {
    type Output: Tuple;
    type Extractor: Extractor<Output = Self::Output>;
    type Tag;

    fn configure(self, cx: &mut Context<'_>) -> crate::app::Result<Self::Extractor>;
}

impl<L, R> PathConfig for Chain<L, R>
where
    L: PathConfig<Tag = self::tags::Incomplete>,
    R: PathConfig,
    L::Output: Combine<R::Output> + Send + 'static,
    R::Output: Send + 'static,
{
    type Output = <L::Output as Combine<R::Output>>::Out;
    type Extractor = Chain<L::Extractor, R::Extractor>;
    type Tag = R::Tag;

    fn configure(self, cx: &mut Context<'_>) -> crate::app::Result<Self::Extractor> {
        let left = self.left.configure(cx)?;
        let right = self.right.configure(cx)?;
        Ok(Chain::new(left, right))
    }
}

impl PathConfig for String {
    type Output = ();
    type Extractor = ();
    type Tag = self::tags::Incomplete;

    fn configure(self, cx: &mut Context<'_>) -> crate::app::Result<Self::Extractor> {
        // TODO: validatation
        cx.components.push(UriComponent::Static(self));
        Ok(())
    }
}

impl<'a> PathConfig for &'a str {
    type Output = ();
    type Extractor = ();
    type Tag = self::tags::Incomplete;

    fn configure(self, cx: &mut Context<'_>) -> crate::app::Result<Self::Extractor> {
        // TODO: validatation
        cx.components.push(UriComponent::Static(self.to_owned()));
        Ok(())
    }
}

/// Creates a `PathConfig` that appends a trailing slash to the path.
pub fn slash() -> Slash {
    Slash(())
}

#[derive(Debug)]
pub struct Slash(());

impl PathConfig for Slash {
    type Output = ();
    type Extractor = ();
    type Tag = self::tags::Completed;

    fn configure(self, cx: &mut Context<'_>) -> crate::app::Result<Self::Extractor> {
        cx.components.push(UriComponent::Slash);
        Ok(())
    }
}

/// Creates a `PathConfig` that appends a parameter with the specified name to the path.
pub fn param<T>(name: impl Into<String>) -> Param<T>
where
    T: FromPercentEncoded,
{
    Param {
        name: name.into(),
        _marker: PhantomData,
    }
}

#[derive(Debug)]
pub struct Param<T> {
    name: String,
    _marker: PhantomData<fn() -> T>,
}

impl<T> PathConfig for Param<T>
where
    T: FromPercentEncoded + Send + 'static,
{
    type Output = (T,);
    type Extractor = Self;
    type Tag = self::tags::Incomplete;

    fn configure(self, cx: &mut Context<'_>) -> crate::app::Result<Self::Extractor> {
        // TODO: validatation
        cx.components
            .push(UriComponent::Param(self.name.clone(), ':'));
        Ok(self)
    }
}

impl<T> Param<T>
where
    T: FromPercentEncoded + Send + 'static,
{
    fn extract_inner(&self, input: &mut Input<'_>) -> Result<(T,), crate::Error> {
        let params = input
            .params
            .as_ref()
            .ok_or_else(|| crate::error::internal_server_error("missing Params"))?;
        let s = params
            .name(&self.name)
            .ok_or_else(|| crate::error::internal_server_error("invalid paramter name"))?;
        T::from_percent_encoded(unsafe { PercentEncoded::new_unchecked(s) })
            .map(|x| (x,))
            .map_err(Into::into)
    }
}

impl<T> Extractor for Param<T>
where
    T: FromPercentEncoded + Send + 'static,
{
    type Output = (T,);
    type Error = crate::error::Error;
    type Future = futures01::future::FutureResult<Self::Output, Self::Error>;

    fn extract(&self, input: &mut Input<'_>) -> Self::Future {
        futures01::future::result(self.extract_inner(input))
    }
}

/// Creates a `PathConfig` that appends a *catch-all* parameter with the specified name to the path.
pub fn catch_all<T>(name: impl Into<String>) -> CatchAll<T>
where
    T: FromPercentEncoded,
{
    CatchAll {
        name: name.into(),
        _marker: PhantomData,
    }
}

#[derive(Debug)]
pub struct CatchAll<T> {
    name: String,
    _marker: PhantomData<fn() -> T>,
}

impl<T> PathConfig for CatchAll<T>
where
    T: FromPercentEncoded + Send + 'static,
{
    type Output = (T,);
    type Extractor = Self;
    type Tag = self::tags::Completed;

    fn configure(self, cx: &mut Context<'_>) -> crate::app::Result<Self::Extractor> {
        // TODO: validatation
        cx.components
            .push(UriComponent::Param(self.name.clone(), '*'));
        Ok(self)
    }
}

impl<T> CatchAll<T>
where
    T: FromPercentEncoded + Send + 'static,
{
    fn extract_inner(&self, input: &mut Input<'_>) -> Result<(T,), crate::Error> {
        let params = input
            .params
            .as_ref()
            .ok_or_else(|| crate::error::internal_server_error("missing Params"))?;
        let s = params
            .catch_all()
            .ok_or_else(|| crate::error::internal_server_error("invalid paramter name"))?;
        T::from_percent_encoded(unsafe { PercentEncoded::new_unchecked(s) })
            .map(|x| (x,))
            .map_err(Into::into)
    }
}

impl<T> Extractor for CatchAll<T>
where
    T: FromPercentEncoded + Send + 'static,
{
    type Output = (T,);
    type Error = crate::error::Error;
    type Future = futures01::future::FutureResult<Self::Output, Self::Error>;

    fn extract(&self, input: &mut Input<'_>) -> Self::Future {
        futures01::future::result(self.extract_inner(input))
    }
}

/// A macro for generating the code that creates a [`Path`] from the provided tokens.
///
/// [`Path`]: ./app/config/route/struct.Path.html
#[macro_export]
macro_rules! path {
    (/) => ( $crate::app::config::route::Path::root() );
    (*) => ( $crate::app::config::route::Path::asterisk() );
    ($(/ $s:tt)+) => ( $crate::app::config::route::Path::create($crate::chain!($($s),*)).unwrap() );
    ($(/ $s:tt)+ /) => ( $crate::app::config::route::Path::create($crate::chain!($($s),*, $crate::app::config::route::slash())).unwrap() );
}

#[derive(Debug)]
pub struct Path<E: Extractor = ()> {
    uri: Option<Uri>,
    extractor: E,
}

impl Path<()> {
    pub fn root() -> Self {
        Self {
            uri: Some(Uri::root()),
            extractor: (),
        }
    }

    pub fn asterisk() -> Self {
        Self {
            uri: None,
            extractor: (),
        }
    }

    pub fn create<T>(config: T) -> crate::app::Result<Path<T::Extractor>>
    where
        T: PathConfig,
    {
        let mut cx = Context {
            components: vec![],
            _marker: PhantomData,
        };
        let extractor = config.configure(&mut cx)?;

        let mut uri = Uri::root();
        for component in cx.components {
            uri.push(component)?;
        }

        Ok(Path {
            uri: Some(uri),
            extractor,
        })
    }
}

impl<E> Path<E>
where
    E: Extractor,
{
    /// Appends a supplemental `Extractor` to this path.
    pub fn extract<E2>(self, other: E2) -> Path<Chain<E, E2>>
    where
        E2: Extractor,
        E::Output: Combine<E2::Output> + Send + 'static,
        E2::Output: Send + 'static,
    {
        Path {
            uri: self.uri,
            extractor: Chain::new(self.extractor, other),
        }
    }

    /// Finalize the configuration in this route and creates the instance of `Route`.
    pub fn to<T>(self, endpoint: T) -> Route<self::handler::RouteHandler<E, T>>
    where
        E: Send + Sync + 'static,
        T: Endpoint<E::Output> + Send + Sync + 'static,
    {
        let Self { uri, extractor, .. } = self;

        let allowed_methods = endpoint.allowed_methods();
        let extractor = Arc::new(extractor);
        let endpoint = Arc::new(endpoint);

        Route {
            uri,
            handler: self::handler::RouteHandler {
                endpoint,
                extractor,
                allowed_methods,
            },
        }
    }
}

mod handler {
    use {
        crate::{
            endpoint::{Endpoint, EndpointAction},
            error::Error,
            extractor::Extractor,
            handler::{AllowedMethods, Handle, Handler},
            input::Input,
        },
        futures01::{try_ready, Future, Poll},
        http::StatusCode,
        std::sync::Arc,
    };

    #[derive(Debug)]
    pub struct RouteHandler<E, T> {
        pub(super) allowed_methods: Option<AllowedMethods>,
        pub(super) extractor: Arc<E>,
        pub(super) endpoint: Arc<T>,
    }

    impl<E, T> Handler for RouteHandler<E, T>
    where
        E: Extractor + Send + Sync + 'static,
        T: Endpoint<E::Output> + Send + Sync + 'static,
    {
        type Output = T::Output;
        type Error = Error;
        type Handle = RouteHandle<E, T>;

        #[inline]
        fn allowed_methods(&self) -> Option<&AllowedMethods> {
            self.allowed_methods.as_ref()
        }

        #[inline]
        fn handle(&self) -> Self::Handle {
            RouteHandle {
                extractor: self.extractor.clone(),
                endpoint: self.endpoint.clone(),
                state: RouteHandleState::Init,
            }
        }
    }

    #[allow(missing_debug_implementations)]
    pub struct RouteHandle<E, T>
    where
        E: Extractor,
        T: Endpoint<E::Output>,
    {
        extractor: Arc<E>,
        endpoint: Arc<T>,
        state: RouteHandleState<E, T>,
    }

    #[allow(missing_debug_implementations)]
    enum RouteHandleState<E, T>
    where
        E: Extractor,
        T: Endpoint<E::Output>,
    {
        Init,
        First(E::Future, Option<T::Action>),
        Second(<T::Action as EndpointAction<E::Output>>::Future),
    }

    impl<E, T> Handle for RouteHandle<E, T>
    where
        E: Extractor,
        T: Endpoint<E::Output>,
    {
        type Output = T::Output;
        type Error = Error;

        #[inline]
        fn poll_ready(&mut self, input: &mut Input<'_>) -> Poll<Self::Output, Self::Error> {
            loop {
                self.state = match self.state {
                    RouteHandleState::Init => {
                        let action = self
                            .endpoint
                            .apply(input.request.method())
                            .ok_or_else(|| StatusCode::METHOD_NOT_ALLOWED)?;
                        let extract = self.extractor.extract(input);
                        RouteHandleState::First(extract, Some(action))
                    }
                    RouteHandleState::First(ref mut future, ref mut action) => {
                        let args = try_ready!(future.poll().map_err(Into::into));
                        let future = action.take().unwrap().call(input, args);
                        RouteHandleState::Second(future)
                    }
                    RouteHandleState::Second(ref mut future) => {
                        return future.poll().map_err(Into::into)
                    }
                }
            }
        }
    }
}
