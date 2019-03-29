use {
    exitfailure::ExitFailure,
    std::path::PathBuf,
    tsukuyomi::{endpoint, path, server::Server, App},
};

fn main() -> Result<(), ExitFailure> {
    let app = App::build(|mut scope| {
        // a route that matches the root path.
        scope.at("/")?.to({
            // an endpoint that matches *all* methods with the root path.
            endpoint::reply("Hello, world\n") // replies by cloning a `Responder`.
        })?;

        // a sub-scope with the prefix `/api/v1/`.
        scope.mount("/api/v1/")?.done(|mut scope| {
            // scopes can be nested.
            scope.mount("/posts")?.done(|mut scope| {
                // a route with the path `/api/v1/posts`.
                scope.at("/")?.done(|mut resource| {
                    resource.get().to(endpoint::reply("list_posts"))?; // <-- GET /api/v1/posts
                    resource.post().to(endpoint::reply("add_post"))?; // <-- POST /api/v1/posts
                    resource.to(endpoint::reply("other methods")) // <-- {PUT, DELETE, ...} /api/v1/posts
                })?;

                // A route that captures a parameter from the path.
                scope.at(path!("/:id"))?.to({
                    endpoint::call(|id: i32| {
                        // returns a `Responder`.
                        format!("get_post(id = {})", id)
                    })
                })
            })?;

            scope
                .mount("/user")?
                .done(|mut scope| scope.at("/auth")?.to(endpoint::reply("Authentication")))
        })?;

        // a route that captures a *catch-all* parameter.
        scope
            .at(path!("/static/*path"))?
            .get()
            .to(endpoint::call(|path: PathBuf| {
                // returns a `Responder`.
                tsukuyomi::fs::NamedFile::open(path)
            }))?;

        // A route that matches any path.
        scope.fallback(endpoint::reply("default route"))?;

        Ok(())
    })?;

    let mut server = Server::new(app)?;
    server.bind("127.0.0.1:4000")?;
    server.run_forever();

    Ok(())
}
