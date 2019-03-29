//! Definition of `Endpoint`.

use {
    crate::{
        error::Error,
        extractor::Extractor,
        future::TryFuture,
        generic::{Combine, Func},
        util::{Chain, Never},
    },
    futures01::IntoFuture,
};

/// A trait representing the process to be performed when a route matches.
pub trait Endpoint<T> {
    type Output;
    type Error: Into<Error>;
    type Future: TryFuture<Ok = Self::Output, Error = Self::Error>;

    /// Maps the provided arguments into a `TryFuture`.
    fn apply(&self, args: T) -> Self::Future;
}

/// A function to create an `Endpoint` from the specified components.
pub fn endpoint<T, R>(
    apply: impl Fn(T) -> R,
) -> impl Endpoint<T, Output = R::Ok, Error = R::Error, Future = R>
where
    R: TryFuture,
{
    #[allow(missing_debug_implementations)]
    struct ApplyFn<F> {
        apply: F,
    }

    impl<F, T, R> Endpoint<T> for ApplyFn<F>
    where
        F: Fn(T) -> R,
        R: TryFuture,
    {
        type Output = R::Ok;
        type Error = R::Error;
        type Future = R;

        #[inline]
        fn apply(&self, args: T) -> Self::Future {
            (self.apply)(args)
        }
    }

    ApplyFn { apply }
}

impl<E, T> Endpoint<T> for std::rc::Rc<E>
where
    E: Endpoint<T>,
{
    type Output = E::Output;
    type Error = E::Error;
    type Future = E::Future;

    #[inline]
    fn apply(&self, args: T) -> Self::Future {
        (**self).apply(args)
    }
}

impl<E, T> Endpoint<T> for std::sync::Arc<E>
where
    E: Endpoint<T>,
{
    type Output = E::Output;
    type Error = E::Error;
    type Future = E::Future;

    #[inline]
    fn apply(&self, args: T) -> Self::Future {
        (**self).apply(args)
    }
}

pub fn builder() -> Builder {
    Builder::new()
}

/// A builder of `Endpoint`.
#[derive(Debug)]
pub struct Builder<E: Extractor = ()> {
    extractor: E,
}

impl Builder {
    /// Creates a `Builder` that accepts the all of HTTP methods.
    pub fn new() -> Self {
        Self { extractor: () }
    }
}

impl<E> Builder<E>
where
    E: Extractor,
{
    /// Appends a supplemental `Extractor` to this endpoint.
    pub fn extract<E2>(self, other: E2) -> Builder<Chain<E, E2>>
    where
        E2: Extractor,
        E::Output: Combine<E2::Output>,
    {
        Builder {
            extractor: Chain::new(self.extractor, other),
        }
    }

    /// Creates an endpoint that replies its result immediately.
    pub fn call<T, F>(
        self,
        f: F,
    ) -> impl Endpoint<
        T,
        Output = F::Out,
        Error = E::Error,
        Future = self::call::CallFuture<E, F, T>, // private
    >
    where
        T: Combine<E::Output>,
        F: Func<<T as Combine<E::Output>>::Out> + Clone,
    {
        let extractor = self.extractor;
        endpoint(move |args: T| self::call::CallFuture {
            extract: extractor.extract(),
            f: f.clone(),
            args: Some(args),
        })
    }

    /// Creates an `Endpoint` that replies its result as a `Future`.
    pub fn call_async<T, F, R>(
        self,
        f: F,
    ) -> impl Endpoint<
        T,
        Output = R::Item,
        Error = Error,
        Future = self::call_async::CallAsyncFuture<E, F, R, T>, // private
    >
    where
        T: Combine<E::Output>,
        F: Func<<T as Combine<E::Output>>::Out, Out = R> + Clone,
        R: IntoFuture,
        R::Error: Into<Error>,
    {
        let extractor = self.extractor;
        endpoint(move |args: T| self::call_async::CallAsyncFuture {
            state: self::call_async::State::First(extractor.extract()),
            f: f.clone(),
            args: Some(args),
        })
    }
}

impl<E> Builder<E>
where
    E: Extractor<Output = ()>,
{
    /// Creates an `Endpoint` that replies the specified value.
    pub fn reply<R>(
        self,
        output: R,
    ) -> impl Endpoint<
        (), //
        Output = R,
        Error = E::Error,
        Future = self::call::CallFuture<E, impl Func<(), Out = R>, ()>, // private
    >
    where
        R: Clone,
    {
        self.call(move || output.clone())
    }
}

/// A shortcut to `endpoint::any().call(f)`
#[inline]
pub fn call<T, F>(
    f: F,
) -> impl Endpoint<
    T, //
    Output = F::Out,
    Error = Never,
    Future = self::call::CallFuture<(), F, T>, // private
>
where
    T: Combine<()>,
    F: Func<<T as Combine<()>>::Out> + Clone,
{
    builder().call(f)
}

/// A shortcut to `endpoint::any().call_async(f)`.
pub fn call_async<T, F, R>(
    f: F,
) -> impl Endpoint<
    T,
    Output = R::Item,
    Error = Error,
    Future = self::call_async::CallAsyncFuture<(), F, R, T>, // private
>
where
    T: Combine<()>,
    F: Func<<T as Combine<()>>::Out, Out = R> + Clone,
    R: IntoFuture,
    R::Error: Into<Error>,
{
    builder().call_async(f)
}

/// A shortcut to `endpoint::any().reply(output)`.
#[inline]
pub fn reply<R>(
    output: R,
) -> impl Endpoint<
    (), //
    Output = R,
    Error = Never,
    Future = self::call::CallFuture<(), impl Func<(), Out = R>, ()>,
>
where
    R: Clone,
{
    builder().reply(output)
}

mod call {
    use crate::{
        extractor::Extractor,
        future::{Async, Poll, TryFuture},
        generic::{Combine, Func},
        input::Input,
    };

    #[allow(missing_debug_implementations)]
    pub struct CallFuture<E: Extractor, F, T> {
        pub(super) extract: E::Extract,
        pub(super) f: F,
        pub(super) args: Option<T>,
    }

    impl<E, F, T> TryFuture for CallFuture<E, F, T>
    where
        E: Extractor,
        F: Func<<T as Combine<E::Output>>::Out>,
        T: Combine<E::Output>,
    {
        type Ok = F::Out;
        type Error = E::Error;

        fn poll_ready(&mut self, input: &mut Input<'_>) -> Poll<Self::Ok, Self::Error> {
            let args2 = futures01::try_ready!(self.extract.poll_ready(input));
            let args = self
                .args
                .take()
                .expect("the future has already been polled.");
            Ok(Async::Ready(self.f.call(args.combine(args2))))
        }
    }
}

mod call_async {
    use {
        crate::{
            error::Error,
            extractor::Extractor,
            future::{Poll, TryFuture},
            generic::{Combine, Func},
            input::Input,
        },
        futures01::{Future, IntoFuture},
    };

    #[allow(missing_debug_implementations)]
    pub(super) enum State<Fut1, Fut2> {
        First(Fut1),
        Second(Fut2),
    }

    #[allow(missing_debug_implementations)]
    pub struct CallAsyncFuture<E: Extractor, F, R: IntoFuture, T> {
        pub(super) state: State<E::Extract, R::Future>,
        pub(super) f: F,
        pub(super) args: Option<T>,
    }

    impl<E, F, R, T> TryFuture for CallAsyncFuture<E, F, R, T>
    where
        E: Extractor,
        F: Func<<T as Combine<E::Output>>::Out, Out = R>,
        R: IntoFuture,
        R::Error: Into<Error>,
        T: Combine<E::Output>,
    {
        type Ok = R::Item;
        type Error = Error;

        fn poll_ready(&mut self, input: &mut Input<'_>) -> Poll<Self::Ok, Self::Error> {
            loop {
                self.state = match self.state {
                    State::First(ref mut extract) => {
                        let args2 =
                            futures01::try_ready!(extract.poll_ready(input).map_err(Into::into));
                        let args = self
                            .args
                            .take()
                            .expect("the future has already been polled.");
                        State::Second(self.f.call(args.combine(args2)).into_future())
                    }
                    State::Second(ref mut action) => return action.poll().map_err(Into::into),
                };
            }
        }
    }
}
