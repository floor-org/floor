extern crate futures;
#[macro_use] extern crate nickel;
extern crate rustc_serialize;

use futures::{Future, Stream};
use nickel::hyper::Error;
use nickel::hyper::Chunk;
use std::collections::BTreeMap;
use nickel::status::StatusCode;
use nickel::{BodyTransformer, Nickel, JsonBody, HttpRouter, MediaType, ResponseStream};
use rustc_serialize::json::{Json, ToJson};

#[derive(RustcDecodable, RustcEncodable)]
struct Person {
    first_name: String,
    last_name:  String,
}

impl ToJson for Person {
    fn to_json(&self) -> Json {
        let mut map = BTreeMap::new();
        map.insert("first_name".to_string(), self.first_name.to_json());
        map.insert("last_name".to_string(), self.last_name.to_json());
        Json::Object(map)
    }
}

fn main() {
    let mut server = Nickel::new();

    // try it with curl
    // curl 'http://localhost:6767/a/post/request' -H 'Content-Type: application/json;charset=UTF-8'  --data-binary $'{ "firstname": "John","lastname": "Connor" }'
    server.post("/", middleware! { |request, response|
        let person = try_with!(response, {
            request.json_as::<Person>().map_err(|e| (StatusCode::BadRequest, e))
        });
        format!("Hello {} {}", person.first_name, person.last_name)
    });

    // server.post("/stream/", middleware! {
    //     |request, response|
    //     let person = try_with!(response, {
    //         request.json_future::<Person>().map_err(|e| (StatusCode::BadRequest, e))
    //     });
    //     let body: ResponseStream = Box::new(person.
    //                                         into_stream().
    //                                         and_then(
    //                                             |p_res|
    //                                             match p_res {
    //                                                 Ok(p) => Ok(Chunk::from(format!("Hello {} {}", p.first_name, p.last_name))),
    //                                                 Err(e) => Err(Error::Incomplete),
    //                                             })
    //     );
    //     body
    // });

    // go to http://localhost:6767/your/name to see this route in action
    server.get("/:first/:last", middleware! { |req|
        // These unwraps are safe because they are required parts of the route
        let first_name = req.param("first").unwrap();
        let last_name = req.param("last").unwrap();

        let person = Person {
            first_name: first_name.to_string(),
            last_name: last_name.to_string(),
        };
        person.to_json()
    });

    // go to http://localhost:6767/content-type to see this route in action
    server.get("/raw", middleware! { |_, mut response|
        response.set(MediaType::Json);
        r#"{ "foo": "bar" }"#
    });

    server.listen("127.0.0.1:6767").unwrap();
}
