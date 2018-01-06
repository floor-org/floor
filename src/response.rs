use std::mem;
use std::borrow::Cow;
use std::path::Path;
use std::time::SystemTime;
use serialize::Encodable;
use futures::future::{self, Future};
use futures::stream::Stream;
use futures::sync::oneshot;
use futures_cpupool::CpuPool;
use futures_fs::FsPool;
use hyper::{Chunk, StatusCode};
use hyper::error::Error as HyperError;
use hyper::server::Response as HyperResponse;
use hyper::header::{
    Headers, Date, Server, ContentType, ContentLength, Header
};
use mimes::MediaType;
use scoped_pool::Pool;
use std::io::{self, Write, copy};
use std::fs::File;
use {NickelError, Halt, MiddlewareResult, Responder, Action};
use template_cache::TemplateCache;
use modifier::Modifier;
use plugin::{Extensible, Pluggable};
use typemap::TypeMap;

pub type ResponseStream = Box<Stream<Item=Chunk, Error=HyperError>>;

///A container for the response
pub struct Response<'a, D: 'a = ()> {
    ///the original `hyper::server::Response`
    pub origin: HyperResponse<ResponseStream>,
    pool: Pool,
    cpupool: CpuPool,
    fspool: FsPool,
    templates: &'a TemplateCache,
    data: &'a D,
    map: TypeMap,
    // This should be FnBox, but that's currently unstable
    on_send: Vec<Box<FnMut(&mut Response<'a, D>)>>
}

impl<'a, D> Response<'a, D> {
    pub fn new<'c, 'd>(pool: Pool, cpupool: CpuPool, fspool: FsPool, templates: &'c TemplateCache, data: &'c D) -> Response<'c, D> {
        Response {
            origin: HyperResponse::new(),
            pool: pool,
            cpupool: cpupool,
            fspool: fspool,
            templates: templates,
            data: data,
            map: TypeMap::new(),
            on_send: vec![]
        }
    }

    /// Get the status.
    pub fn status(&self) -> StatusCode {
        self.origin.status()
    }

    /// Set the status.
    pub fn set_status(&mut self, status: StatusCode) {
        self.origin.set_status(status)
    }

    /// Get a mutable reference to the Headers.
    pub fn headers_mut(&mut self) -> &mut Headers {
        self.origin.headers_mut()
    }

    /// Modify the response with the provided data.
    ///
    /// # Examples
    /// ```{rust}
    /// extern crate hyper;
    /// #[macro_use] extern crate nickel;
    ///
    /// use nickel::{Nickel, HttpRouter, MediaType};
    /// use nickel::status::StatusCode;
    /// use hyper::header::Location;
    ///
    /// fn main() {
    ///     let mut server = Nickel::new();
    ///     server.get("/a", middleware! { |_, mut res|
    ///             // set the Status
    ///         res.set(StatusCode::PermanentRedirect)
    ///             // update a Header value
    ///            .set(Location::new("http://nickel.rs".to_string()));
    ///
    ///         ""
    ///     });
    ///
    ///     server.get("/b", middleware! { |_, mut res|
    ///             // setting the content type
    ///         res.set(MediaType::Json);
    ///
    ///         "{'foo': 'bar'}"
    ///     });
    ///
    ///     // ...
    /// }
    /// ```
    pub fn set<T: Modifier<Response<'a, D>>>(&mut self, attribute: T) -> &mut Response<'a, D> {
        attribute.modify(self);
        self
    }

    /// Writes a response and halts middleware processing.
    ///
    /// # Examples
    /// ```{rust}
    /// use nickel::{Request, Response, MiddlewareResult};
    ///
    /// # #[allow(dead_code)]
    /// fn handler<'a, D>(_: &mut Request<D>, res: Response<'a, D>) -> MiddlewareResult<'a, D> {
    ///     res.send("hello world")
    /// }
    /// ```
    #[inline]
    pub fn send<T: Responder<D>>(self, data: T) -> MiddlewareResult<'a, D> {
        data.respond(self)
    }

    /// Writes a file to the output and Halts middleware processing.
    ///
    /// # Examples
    /// ```{rust}
    /// use nickel::{Request, Response, MiddlewareResult};
    /// use std::path::Path;
    ///
    /// # #[allow(dead_code)]
    /// fn handler<'a, D>(_: &mut Request<D>, res: Response<'a, D>) -> MiddlewareResult<'a, D> {
    ///     let favicon = Path::new("/assets/favicon.ico");
    ///     res.send_file(favicon)
    /// }
    /// ```
    pub fn send_file<P:AsRef<Path>>(mut self, path: P) -> MiddlewareResult<'a, D> {
        let path_buf = path.as_ref().to_owned();
        // Chunk the response
        self.origin.headers_mut().remove::<ContentLength>();
        // Determine content type by file extension or default to binary
        let mime = mime_from_filename(&path_buf).unwrap_or(MediaType::Bin);
        self.set_header_fallback(|| ContentType(mime.into()));

        self.start();

        // using futures-fs
        // let stream = self.fspool.read(path_ref.to_owned()).
        //     map(|b| Chunk::from(b)).
        //     map_err(|e| HyperError::from(e));

        // using futures-cpupool
        let stream = self.cpupool.spawn_fn(|| {
            let mut file = match File::open(path_buf) {
                Ok(f) => f,
                Err(e) => { return future::err(e) },
            };
            let mut buf = Vec::new();
            match copy(&mut file, &mut buf) {
                Ok(_) => {
                    eprintln!("Got buf: {:?}", &buf[0..16]);
                    future::ok(buf)
                },
                Err(e) => future::err(e),
            }
        }).into_stream().
            map(|b| Chunk::from(b)).
            map_err(|e| HyperError::from(e));
        let body: ResponseStream = Box::new(stream);
        self.origin.set_body(body);
        Ok(Halt(self))

        // manually using scoped thread pool
        // let (tx, rx) = oneshot::channel();
        // self.pool.scoped(|scope| {
        //     scope.execute(move || {
        //         let mut file = match File::open(path_buf) {
        //             Ok(f) => f,
        //             Err(e) => { tx.send(Err(e)); return; },
        //         };
        //         let mut buf: Vec<u8> = Vec::new();
        //         match copy(&mut file, &mut buf) {
        //             Ok(_) => { tx.send(Ok(buf)); },
        //             Err(e) => { tx.send(Err(e)); },
        //         };
        //     })
        // });
        // let body: ResponseStream = Box::new(rx.
        //                                     into_stream().
        //                                     map_err(|e| HyperError::from(io::Error::new(io::ErrorKind::Other, e))).
        //                                     and_then(|r| match r {
        //                                         Ok(r) => Ok(Chunk::from(r)),
        //                                         Err(e) => Err(HyperError::from(e)),
        //                                     })
        // );
        // self.origin.set_body(body);
        // Ok(Halt(self))
    }

    pub fn set_body(mut self, body: ResponseStream) -> MiddlewareResult<'a, D> {
        self.origin.set_body(body);
        Ok(Halt(self))
    }

    // TODO: This needs to be more sophisticated to return the correct headers
    // not just "some headers" :)
    //
    // Also, it should only set them if not already set.
    fn set_fallback_headers(&mut self) {
        self.set_header_fallback(|| Date(SystemTime::now().into()));
        self.set_header_fallback(|| Server::new("Nickel"));
        self.set_header_fallback(|| ContentType(MediaType::Html.into()));
    }

    /// Return an error with the appropriate status code for error handlers to
    /// provide output for.
    pub fn error<T>(self, status: StatusCode, message: T) -> MiddlewareResult<'a, D>
            where T: Into<Cow<'static, str>> {
        Err(NickelError::new(self, message, status))
    }

    /// Sets the header if not already set.
    ///
    /// If the header is not set then `f` will be called.
    ///
    /// # Examples
    /// ```{rust}
    /// #[macro_use] extern crate nickel;
    /// extern crate hyper;
    ///
    /// use nickel::{Nickel, HttpRouter, MediaType};
    /// use hyper::header::ContentType;
    ///
    /// # #[allow(unreachable_code)]
    /// fn main() {
    ///     let mut server = Nickel::new();
    ///     server.get("/", middleware! { |_, mut res|
    ///         res.set(MediaType::Html);
    ///         res.set_header_fallback(|| {
    ///             panic!("Should not get called");
    ///             ContentType(MediaType::Txt.into())
    ///         });
    ///
    ///         "<h1>Hello World</h1>"
    ///     });
    ///
    ///     // ...
    /// }
    /// ```
    pub fn set_header_fallback<F, H>(&mut self, f: F)
            where H: Header, F: FnOnce() -> H {
        let headers = self.origin.headers_mut();
        if !headers.has::<H>() { headers.set(f()) }
    }

    /// Renders the given template bound with the given data and halts middlware processing.
    ///
    /// # Examples
    /// ```{rust}
    /// use std::collections::HashMap;
    /// use nickel::{Request, Response, MiddlewareResult};
    ///
    /// # #[allow(dead_code)]
    /// fn handler<'a, D>(_: &mut Request<D>, res: Response<'a, D>) -> MiddlewareResult<'a, D> {
    ///     let mut data = HashMap::new();
    ///     data.insert("name", "user");
    ///     res.render("examples/assets/template.tpl", &data)
    /// }
    /// ```
    pub fn render<T, P>(mut self, path: P, data: &T) -> MiddlewareResult<'a, D>
        where T: Encodable + Sync, P: AsRef<Path> + Into<String> {

        let (tx, rx) = oneshot::channel();
        let path_buf = path.as_ref().to_owned();
        let templates = self.templates;
        self.start();
        self.pool.scoped(|scope| {
            scope.execute(move || {
                let mut buf: Vec<u8> = Vec::new();
                match templates.render(path_buf, &mut buf, data) {
                    Ok(()) => { tx.send(Ok(buf)); },
                    Err(e) => { tx.send(Err(io::Error::new(io::ErrorKind::Other, e))); },
                };
            })
        });
        let body: ResponseStream = Box::new(rx.
                                            into_stream().
                                            map_err(|e| HyperError::from(io::Error::new(io::ErrorKind::Other, e))).
                                            and_then(|r| match r {
                                                Ok(r) => Ok(Chunk::from(r)),
                                                Err(e) => Err(HyperError::from(e)),
                                            })
        );
        self.origin.set_body(body);
        Ok(Halt(self))
    }

    // Todo: migration cleanup
    //
    // hyper::Response no longer has a start() method. The api has
    // changed a lot, so this may not longer be necessary.
    //
    // What we are still doing is running the on_send mthods, and
    // setting fallback headers. Do we need this dedicated method in
    // the workflow to make sure that happens?
    pub fn start(&mut self) {
        let on_send = mem::replace(&mut self.on_send, vec![]);
        for mut f in on_send.into_iter().rev() {
            // TODO: Ensure `f` doesn't call on_send again
            f(self)
        }

        // Set fallback headers last after everything runs, if we did this before as an
        // on_send then it would possibly set redundant things.
        self.set_fallback_headers();
    }

    pub fn server_data(&self) -> &'a D {
        &self.data
    }

    pub fn on_send<F>(&mut self, f: F)
            where F: FnMut(&mut Response<'a, D>) + 'static {
        self.on_send.push(Box::new(f))
    }

    /// Pass execution off to another Middleware
    ///
    /// When returned from a Middleware, it allows computation to continue
    /// in any Middleware queued after the active one.
    pub fn next_middleware(self) -> MiddlewareResult<'a, D> {
        Ok(Action::Continue(self))
    }
}

// impl<'a, 'b, D> Write for Response<'a, D> {
//     #[inline(always)]
//     // Todo: migration cleanup
//     //
//     // Should be easy, just change to a simple future::Stream
//     fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
//         // self.origin.write(buf)
//         Ok(0)
//     }

//     #[inline(always)]
//     // Todo: migration cleanup
//     //
//     // Should be easy, may not even be needed
//     fn flush(&mut self) -> io::Result<()> {
//         // self.origin.flush()
//         Ok(())
//     }
// }

impl<'a, 'b, D> Response<'a, D> {
    /// In the case of an unrecoverable error while a stream is already in
    /// progress, there is no standard way to signal to the client that an
    /// error has occurred. `bail` will drop the connection and log an error
    /// message.
    pub fn bail<T>(self, message: T) -> MiddlewareResult<'a, D>
            where T: Into<Cow<'static, str>> {
        let _ = self.end();
        unsafe { Err(NickelError::without_response(message)) }
    }

    /// Flushes all writing of a response to the client.
    // Todo: migration cleanup
    //
    // Should be easy, may not even be needed
    pub fn end(self) -> io::Result<()> {
        // self.origin.end()
        Ok(())
    }
}

impl <'a, D> Response<'a, D> {
    /// The headers of this response.
    pub fn headers(&self) -> &Headers {
        self.origin.headers()
    }

    pub fn data(&self) -> &'a D {
        &self.data
    }
}

impl<'a, D> Extensible for Response<'a, D> {
    fn extensions(&self) -> &TypeMap {
        &self.map
    }

    fn extensions_mut(&mut self) -> &mut TypeMap {
        &mut self.map
    }
}

impl<'a, D> Pluggable for Response<'a, D> {}

fn mime_from_filename<P: AsRef<Path>>(path: P) -> Option<MediaType> {
    path.as_ref()
        .extension()
        .and_then(|os| os.to_str())
        // Lookup mime from file extension
        .and_then(|s| s.parse().ok())
}

#[test]
fn matches_content_type () {
    assert_eq!(Some(MediaType::Txt), mime_from_filename("test.txt"));
    assert_eq!(Some(MediaType::Json), mime_from_filename("test.json"));
    assert_eq!(Some(MediaType::Bin), mime_from_filename("test.bin"));
}

mod modifier_impls {
    use hyper::header::*;
    use hyper::StatusCode;
    use modifier::Modifier;
    use {Response, MediaType};

    impl<'a, D> Modifier<Response<'a, D>> for StatusCode {
        fn modify(self, res: &mut Response<'a, D>) {
            res.set_status(self)
        }
    }

    impl<'a, D> Modifier<Response<'a, D>> for MediaType {
        fn modify(self, res: &mut Response<'a, D>) {
            ContentType(self.into()).modify(res)
        }
    }

    macro_rules! header_modifiers {
        ($($t:ty),+) => (
            $(
                impl<'a, D> Modifier<Response<'a, D>> for $t {
                    fn modify(self, res: &mut Response<'a, D>) {
                        res.headers_mut().set(self)
                    }
                }
            )+
        )
    }

    header_modifiers! {
        Accept,
        AccessControlAllowHeaders,
        AccessControlAllowMethods,
        AccessControlAllowOrigin,
        AccessControlMaxAge,
        AccessControlRequestHeaders,
        AccessControlRequestMethod,
        AcceptCharset,
        AcceptEncoding,
        AcceptLanguage,
        AcceptRanges,
        Allow,
        Authorization<Basic>,
        Authorization<Bearer>,
        Authorization<String>,
        CacheControl,
        Cookie,
        Connection,
        ContentEncoding,
        ContentLanguage,
        ContentLength,
        ContentType,
        Date,
        ETag,
        Expect,
        Expires,
        From,
        Host,
        IfMatch,
        IfModifiedSince,
        IfNoneMatch,
        IfRange,
        IfUnmodifiedSince,
        LastModified,
        Location,
        Pragma,
        Referer,
        Server,
        SetCookie,
        TransferEncoding,
        Upgrade,
        UserAgent,
        Vary
    }
}
