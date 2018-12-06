mod integration_tests;

#[test]
#[should_panic]
fn test_catch_unwind() {
    fn inner() -> tsukuyomi::test::Result<()> {
        let mut server = tsukuyomi::App::builder()
            .with(
                tsukuyomi::app::route::root() //
                    .reply(|| -> &'static str { panic!("explicit panic") }),
            ) //
            .build_server()?
            .into_test_server()?;

        server.perform("/")?;

        Ok(())
    }

    if let Err(err) = inner() {
        eprintln!("unexpected error: {:?}", err);
    }
}
