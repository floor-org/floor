#[macro_use] extern crate nickel;
extern crate regex;
extern crate rustc_serialize;
extern crate hyper;

use std::io::Write;
use nickel::status::StatusCode::{self, NotFound};
use nickel::{
    Nickel, NickelError, Continue, Halt, Response, MediaType, QueryString,
    JsonBody, StaticFilesHandler, MiddlewareResult, HttpRouter, Action
};
use regex::Regex;
use hyper::header::Location;

#[derive(RustcDecodable, RustcEncodable)]
struct Person {
    firstname: String,
    lastname:  String,
}

//this is an example middleware function that just logs each request
fn logger<'a, 'k, D>(response: Response<'a, 'k, D>) -> MiddlewareResult<'a, 'k, D> {
    println!("logging request: {:?}", response.request.origin.uri);
    Ok(Continue(response))
}

//this is how to overwrite the default error handler to handle 404 cases with a custom view
fn custom_404<'a, D>(err: &mut NickelError<D>) -> Action {
    if let Some(ref mut res) = err.response_mut() {
        if res.status() == NotFound {
            let _ = res.write_all(b"<h1>Call the police!</h1>");
            return Halt(())
        }
    }

    Continue(())
}

fn main() {
    let mut server = Nickel::new();

    // middleware is optional and can be registered with `utilize`
    server.utilize(logger);

    // go to http://localhost:6767/thoughtram_logo_brain.png to see static file serving in action
    server.utilize(StaticFilesHandler::new("examples/assets/"));

    let hello_regex = Regex::new("/hello/(?P<name>[a-zA-Z]+)").unwrap();

    // The return type for a route can be anything that implements `Responder`
    server.utilize(router!(
        // go to http://localhost:6767/user/4711 to see this route in action
        get "/user/:userid" => |response| {
            // returning a String
            format!("This is user: {}", response.param("userid").unwrap())
        }

        // go to http://localhost:6767/no_alloc/4711 to see this route in action
        // get "/no_alloc/:userid" => |response| {
        //     // returning a slice of T where T: Display
        //     &["This is user: ", response.param("userid").unwrap()][..]
        // }

        // go to http://localhost:6767/bar to see this route in action
        get "/bar" => {
            // returning a http status code and a static string
            (200u16, "This is the /bar handler")
        }

        // go to http://localhost:6767/content-type to see this route in action
        get "/content-type" => |mut response| {
            response.set(MediaType::Json);
            "{'foo':'bar'}"
        }

        // go to http://localhost:6767/hello/moomah to see this route in action
        get hello_regex => |response| {
            format!("Hello {}", response.param("name").unwrap())
        }

        // go to http://localhost:6767/redirect to see this route in action
        get "/redirect" => |mut response| {
            response.set(Location("http://nickel.rs".into()));

            StatusCode::PermanentRedirect
        }

        // go to http://localhost:6767/private to see this route in action
        get "/private" => {
            // returning a typed http status and a response body
            (StatusCode::Unauthorized, "This is a private place")
        }

        // go to http://localhost:6767/some/crazy/route to see this route in action
        get "/some/*/route" => {
            // returning a static string
            "This matches /some/crazy/route but not /some/super/crazy/route"
        }

        // go to http://localhost:6767/a/some/crazy/route to see this route in action
        get "/a/**/route" => {
            "This matches /a/crazy/route and also /a/super/crazy/route"
        }

        // try it with curl
        // curl 'http://localhost:6767/a/post/request' -H 'Content-Type: application/json;charset=UTF-8'  --data-binary $'{ "firstname": "John","lastname": "Connor" }'
        post "/a/post/request" => |mut response| {
            let person = response.request.json_as::<Person>().unwrap();
            format!("Hello {} {}", person.firstname, person.lastname)
        }

        // try calling http://localhost:6767/query?foo=bar
        get "/query" => |mut response| {
            let text = {
                let query = response.request.query();
                let foo = query.get("foo").unwrap_or("This is only a default value");
                let bar = query.get("bar").unwrap_or("This is only a default value");
                format!("<p>Your foo values in the query string are: {:?}\
                         <p>Your bar values are: {:?}",
                         foo, bar);
            };

            text
        }
    ));

    // issue #20178
    let custom_handler: fn(&mut NickelError<()>) -> Action = custom_404;

    server.handle_error(custom_handler);

    println!("Running server!");
    server.listen("127.0.0.1:6767");
}
