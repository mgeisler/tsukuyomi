extern crate ganymede;
extern crate http;
extern crate pretty_env_logger;

use ganymede::router::{Route, RouterContext};
use ganymede::{App, Context, Error};
use http::Method;

fn welcome(_cx: &Context, _rcx: &mut RouterContext) -> Result<&'static str, Error> {
    Ok("Hello")
}

fn main() -> ganymede::app::Result<()> {
    pretty_env_logger::init();
    App::builder()
        .mount(vec![Route::new("/", Method::GET, welcome)])
        .serve()
}
