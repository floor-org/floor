//!Router asigns handlers to paths and resolves them per request

#[cfg(test)]
use http::method;
use http::method::Method;
use http::server::request::{AbsolutePath};
use regex::Regex;
use std::collections::hashmap::HashMap;
use request::Request;
use response::Response;
use middleware::{Middleware, Halt, Continue, MiddlewareResult};

pub use self::http_router::HttpRouter;
mod http_router;

pub trait RequestHandler : Sync + Send {
    fn handle(&self, &Request, &mut Response);
}

/// A Route is the basic data structure that stores both the path
/// and the handler that gets executed for the route.
/// The path can contain variable pattern such as `user/:userid/invoices`
pub struct Route {
    pub path: String,
    pub method: Method,
    pub handler: Box<RequestHandler + Send + Sync + 'static>,
    pub variables: HashMap<String, uint>,
    matcher: Regex
}

impl RequestHandler for fn(request: &Request, response: &mut Response) {
    fn handle(&self, req: &Request, res: &mut Response) {
        (*self)(req, res)
    }
}

/// A RouteResult is what the router returns when `match_route` is called.
/// It contains the matched `route` and also a `params` property holding
/// a HashMap with the keys being the variable names and the value being the
/// evaluated string
pub struct RouteResult<'a> {
    pub route: &'a Route,
    params: Vec<String>
}

impl<'a> RouteResult<'a> {
    pub fn param(&self, key: &str) -> &str {
        let idx = match self.route.variables.find_equiv(&key) {
            Some(idx) => idx,
            None => {
                fail!("Unknown param '{}' for route '{}'", key, self.route.path)
            }
        };

        self.params[*idx].as_slice()
    }
}

/// The path_utils collects some small helper methods that operate on the path
mod path_utils {
    use regex::Regex;
    use std::collections::hashmap::HashMap;

    // matches named variables (e.g. :userid)
    static REGEX_VAR_SEQ: Regex                 = regex!(r":([,a-zA-Z0-9_-]*)");
    static VAR_SEQ:&'static str                 = "[,a-zA-Z0-9_-]*";
    static VAR_SEQ_WITH_SLASH:&'static str      = "[,/a-zA-Z0-9_-]*";
    static VAR_SEQ_WITH_CAPTURE:&'static str    = "([,a-zA-Z0-9%_-]*)";
    // matches request params (e.g. ?foo=true&bar=false)
    static REGEX_PARAM_SEQ:&'static str         = "(\\?[a-zA-Z0-9%_=&-]*)?";
    static REGEX_START:&'static str             = "^";
    static REGEX_END:&'static str               = "$";
    pub fn create_regex (route_path: &str) -> Regex {
        let updated_path =
            route_path.to_string()
                      // first mark all double wildcards for replacement.
                      // We can't directly replace them since the replacement
                      // does contain the * symbol as well, which would get
                      // overwritten with the next replace call
                      .replace("**", "___DOUBLE_WILDCARD___")
                      // then replace the regular wildcard symbols (*) with the
                      // appropriate regex
                      .replace("*", VAR_SEQ)
                      // now replace the previously marked double wild cards (**)
                      .replace("___DOUBLE_WILDCARD___", VAR_SEQ_WITH_SLASH);

        // then replace the variable symbols (:variable) with the appropriate regex
        let result = [REGEX_START,
                      REGEX_VAR_SEQ.replace_all(updated_path.as_slice(),
                                                VAR_SEQ_WITH_CAPTURE)
                                   .as_slice(),
                      REGEX_PARAM_SEQ,
                      REGEX_END].concat();

        Regex::new(result.as_slice()).ok().unwrap()
    }

    pub fn get_variable_info (route_path: &str) -> HashMap<String, uint> {
        REGEX_VAR_SEQ.captures_iter(route_path)
             .enumerate()
             .map(|(i, matched)| (matched.at(1).to_string(), i))
             .collect()
    }
}

/// The Router's job is it to hold routes and to resolve them later against
/// concrete URLs. The router is also a regular middleware and needs to be
/// added to the middleware stack with `server.utilize(router)`.
pub struct Router{
    routes: Vec<Route>,
}

impl<'a> Router {
    pub fn new () -> Router {
        Router {
            routes: Vec::new()
        }
    }

    pub fn match_route(&'a self, method: &Method, path: &str)
                        -> Option<RouteResult<'a>> {
        self.routes.iter().find(|item| {
            item.method == *method
            && item.matcher.is_match(path)
        }).map(|route| {
            let vec = match route.matcher.captures(path) {
                Some(captures) => {
                    range(0, route.variables.len()).map(|pos|
                        captures.at(pos + 1).to_string()
                    ).collect()
                },
                None => vec![],
            };

            RouteResult {
                route: route,
                params: vec
            }
        })
    }
}

impl HttpRouter for Router {
    fn add_route<H: RequestHandler>(&mut self, method: Method, path: &str, handler: H) {
        static FORMAT_VAR: &'static str = ":format";

        let with_format = if path.contains(FORMAT_VAR) {
            path.to_string()
        } else {
            format!("{}(\\.{})?", path, FORMAT_VAR)
        };

        let matcher = path_utils::create_regex(with_format[]);
        let variable_infos = path_utils::get_variable_info(with_format[]);

        let route = Route {
            path: with_format,
            method: method,
            matcher: matcher,
            handler: box handler,
            variables: variable_infos
        };
        self.routes.push(route);
    }
}

impl Middleware for Router {
    fn invoke<'a, 'b>(&'a self, req: &mut Request<'b, 'a>, res: &mut Response)
                        -> MiddlewareResult {
        match req.origin.request_uri {
            AbsolutePath(ref url) => {
                match self.match_route(&req.origin.method, url.as_slice()) {
                    Some(route_result) => {
                        res.origin.status = ::http::status::Ok;
                        let handler = &route_result.route.handler;
                        req.route_result = Some(route_result);
                        handler.handle(req, res);
                        Ok(Halt)
                    },
                    None => Ok(Continue)
                }
            },
            _ => Ok(Continue)
        }
    }
}

#[test]
fn creates_map_with_var_variable_infos () {
    let map = path_utils::get_variable_info("foo/:uid/bar/:groupid");

    assert_eq!(map.len(), 2);
    assert_eq!(map["uid".to_string()], 0);
    assert_eq!(map["groupid".to_string()], 1);
}

#[test]
fn creates_regex_with_captures () {
    let regex = path_utils::create_regex("foo/:uid/bar/:groupid");
    let caps = regex.captures("foo/4711/bar/5490").unwrap();

    assert_eq!(caps.at(1), "4711");
    assert_eq!(caps.at(2), "5490");

    let regex = path_utils::create_regex("foo/*/:uid/bar/:groupid");
    let caps = regex.captures("foo/test/4711/bar/5490").unwrap();

    assert_eq!(caps.at(1), "4711");
    assert_eq!(caps.at(2), "5490");

    let regex = path_utils::create_regex("foo/**/:uid/bar/:groupid");
    let caps = regex.captures("foo/test/another/4711/bar/5490").unwrap();

    assert_eq!(caps.at(1), "4711");
    assert_eq!(caps.at(2), "5490");
}

#[test]
fn creates_valid_regex_for_routes () {
    let regex1 = path_utils::create_regex("foo/:uid/bar/:groupid");
    let regex2 = path_utils::create_regex("foo/*/bar");
    let regex3 = path_utils::create_regex("foo/**/bar");

    assert_eq!(regex1.is_match("foo/4711/bar/5490"), true);
    assert_eq!(regex1.is_match("foo/4711/bar/5490?foo=true&bar=false"), true);
    assert_eq!(regex1.is_match("foo/4711/bar"), false);
    assert_eq!(regex1.is_match("foo/4711/bar?foo=true&bar=false"), false);
    assert_eq!(regex1.is_match("foo/4711/bar/test%20spacing"), true);
    assert_eq!(regex1.is_match("foo/4711/bar/5281?foo=test%20spacing&bar=false"), true);

    assert_eq!(regex2.is_match("foo/4711/bar"), true);
    assert_eq!(regex2.is_match("foo/4711/barr"), false);
    assert_eq!(regex2.is_match("foo/4711/bar?foo=true&bar=false"), true);
    assert_eq!(regex2.is_match("foo/4711/4712/bar"), false);
    assert_eq!(regex2.is_match("foo/4711/4712/bar?foo=true&bar=false"), false);

    assert_eq!(regex3.is_match("foo/4711/bar"), true);
    assert_eq!(regex3.is_match("foo/4711/bar?foo=true&bar=false"), true);
    assert_eq!(regex3.is_match("foo/4711/4712/bar"), true);
    assert_eq!(regex3.is_match("foo/4711/4712/bar?foo=true&bar=false"), true);

    //ensure that this works with commas too
    assert_eq!(regex1.is_match("foo/4711/bar/5490,1234"), true);
    assert_eq!(regex1.is_match("foo/4711/bar/5490,1234?foo=true&bar=false"), true);
    assert_eq!(regex1.is_match("foo/4711/bar"), false);
    assert_eq!(regex1.is_match("foo/4711/bar?foo=1,2,3&bar=false"), false);
}

#[test]
fn can_match_var_routes () {
    let route_store = &mut Router::new();

    fn handler (_request: &Request, response: &mut Response) -> () {
        let _ = response.origin.write("hello from foo".as_bytes());
    };

    route_store.add_route(method::Get, "/foo/:userid", handler);
    route_store.add_route(method::Get, "/bar", handler);
    route_store.add_route(method::Get, "/file/:format/:file", handler);

    let route_result = route_store.match_route(&method::Get, "/foo/4711").unwrap();
    let route = route_result.route;

    assert_eq!(route_result.param("userid"), "4711");

    // assert the route has identified the variable
    assert_eq!(route.variables.len(), 2);
    assert_eq!(route.variables["userid".to_string()], 0);
    // routes have an implicit format variable
    assert_eq!(route.variables["format".to_string()], 1);

    let route_result = route_store.match_route(&method::Get, "/bar/4711");
    assert!(route_result.is_none());

    let route_result = route_store.match_route(&method::Get, "/foo");
    assert!(route_result.is_none());

    // ensure that this will work with commas too
    let route_result = route_store.match_route(&method::Get, "/foo/123,456");
    assert!(route_result.is_some());

    let route_result = route_result.unwrap();
    assert_eq!(route_result.param("userid"), "123,456");

    // ensure that this will work with spacing too
    let route_result = route_store.match_route(&method::Get, "/foo/John%20Doe");
    assert!(route_result.is_some());

    let route_result = route_result.unwrap();
    assert_eq!(route_result.param("userid"), "John%20Doe");

    // check for optional format param
    let route_result = route_store.match_route(&method::Get, "/foo/John%20Doe.json");
    assert!(route_result.is_some());

    let route_result = route_result.unwrap();
    assert_eq!(route_result.param("userid"), "John%20Doe");
    assert_eq!(route_result.param("format"), ".json");

    // ensure format works with queries
    let route_result = route_store.match_route(&method::Get,
                                               "/foo/5490,1234.csv?foo=true&bar=false");
    assert!(route_result.is_some());

    let route_result = route_result.unwrap();
    // NOTE: `.param` doesn't cover query params currently
    assert_eq!(route_result.param("userid"), "5490,1234");
    assert_eq!(route_result.param("format"), ".csv");

    // ensure format works if defined by user
    let route_result = route_store.match_route(&method::Get,
                                               "/file/markdown/something?foo=true");
    assert!(route_result.is_some());

    let route_result = route_result.unwrap();
    // NOTE: `.param` doesn't cover query params currently
    assert_eq!(route_result.param("file"), "something");
    assert_eq!(route_result.param("format"), "markdown");
}