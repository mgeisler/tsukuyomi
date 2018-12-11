use {
    cookie::Cookie,
    tsukuyomi::{
        app::config::route::Route, //
        chain,
        server::Server,
        App,
    },
};

#[test]
fn enable_manage_cookies() -> tsukuyomi::test::Result<()> {
    let mut server = App::configure(chain![
        Route::from_parts(
            "/first",
            tsukuyomi::handler::ready(|input| -> tsukuyomi::Result<_> {
                input.cookies.jar()?.add(Cookie::new("session", "xxxx"));
                Ok("")
            })
        )?,
        Route::from_parts(
            "/second",
            tsukuyomi::handler::ready(|input| -> tsukuyomi::Result<_> {
                assert!(input.cookies.jar()?.get("session").is_some());
                Ok("")
            })
        ),
    ])
    .map(Server::new)?
    .into_test_server()?;

    let mut session = server.new_session()?.save_cookies(true);
    let _ = session.perform("/first")?;
    let _ = session.perform("/second")?;

    Ok(())
}

#[test]
fn disable_manage_cookies() -> tsukuyomi::test::Result<()> {
    let mut server = App::configure(chain![
        Route::from_parts(
            "/first",
            tsukuyomi::handler::ready(|input| -> tsukuyomi::Result<_> {
                input.cookies.jar()?.add(Cookie::new("session", "xxxx"));
                Ok("")
            })
        ),
        Route::from_parts(
            "/second",
            tsukuyomi::handler::ready(|input| -> tsukuyomi::Result<_> {
                assert!(input.cookies.jar()?.get("session").is_none());
                Ok("")
            })
        ),
    ])
    .map(Server::new)?
    .into_test_server()?;

    let mut session = server.new_session()?;
    let _ = session.perform("/first")?;
    let _ = session.perform("/second")?;

    Ok(())
}
