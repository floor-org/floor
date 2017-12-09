extern crate rustc_serialize;
extern crate hyper;
extern crate unicase;
extern crate tokio_core;
extern crate futures;
// HACK: integration_testing example refers to `nickel::foo`
// and this import helps that resolve rather than requiring `self::nickel::foo`
// which is an oddity due to the include method, which is used as tests in examples
// don't get executed otherwise.
#[macro_use]
extern crate nickel;

#[macro_use] extern crate lazy_static;

mod util;

mod examples {
    mod hello_world;
    mod mount;
    mod route_data;
    mod routing;
    mod template;
    mod moved_ownership;
    mod chaining;
    mod json; // Todo: migration cleanup - freezes during test run
    mod query_string;
    mod regex_route;
    mod custom_error_handler;
    mod static_files;
    mod enable_cors;
    // mod form_data; // Todo: fixme
    // mod integration_testing; // Todo: fixme

    #[cfg(feature = "ssl")]
    mod https;
}
