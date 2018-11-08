#![allow(missing_docs)]

pub use crate::app::route::Builder;
pub use crate::route_expr as route;

#[macro_export(local_inner_macros)]
macro_rules! route_expr {
    ($uri:expr) => {{
        enum __Dummy {}
        impl __Dummy {
            route_expr_impl!($uri);
        }
        __Dummy::route()
    }};
    ($uri:expr, methods = [$($methods:expr),*]) => {
        route_expr!($uri)
            $( .method($methods) )*
    };
    () => {
        $crate::route::index()
    };
}

#[inline]
pub fn route<T>(uri: T) -> Builder<()>
where
    T: AsRef<str>,
{
    Builder::new(()).uri(uri)
}

#[inline]
pub fn index() -> Builder<()> {
    self::route("/")
}

macro_rules! define_route {
    ($($method:ident => $METHOD:ident,)*) => {$(
        pub fn $method<T>(uri: T) -> Builder<()>
        where
            T: AsRef<str>,
        {
            self::route(uri).method(http::Method::$METHOD)
        }
    )*}
}

define_route! {
    get => GET,
    post => POST,
    put => PUT,
    delete => DELETE,
    head => HEAD,
    options => OPTIONS,
    connect => CONNECT,
    patch => PATCH,
    trace => TRACE,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::Extractor;

    fn generated() -> Builder<impl Extractor<Output = (u32, String)>> {
        self::route("/:id/:name")
            .with(crate::extractor::param::pos(0))
            .with(crate::extractor::param::pos(1))
    }

    #[test]
    #[ignore]
    fn compiletest1() {
        drop(
            crate::app(|scope| {
                scope.route(generated().reply(|id: u32, name: String| {
                    drop((id, name));
                    "dummy"
                }));
            }).expect("failed to construct App"),
        );
    }

    #[test]
    #[ignore]
    fn compiletest2() {
        drop(
            crate::app(|scope| {
                scope.route(generated().with(crate::extractor::body::plain()).reply(
                    |id: u32, name: String, body: String| {
                        drop((id, name, body));
                        "dummy"
                    },
                ));
            }).expect("failed to construct App"),
        );
    }
}