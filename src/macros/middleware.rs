/// Macro to reduce the boilerplate required for using unboxed
/// closures as `Middleware` due to current type inference behaviour.
///
/// In future, the macro should hopefully be able to be removed while
/// having minimal changes to the closure's code.
///
/// # Examples
/// ```rust,no_run
/// # #[macro_use] extern crate nickel;
/// # fn main() {
/// use nickel::{Nickel, HttpRouter};
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// let mut server = Nickel::new();
///
/// // Some shared resource between requests, must be `Sync + Send`
/// let visits = AtomicUsize::new(0);
///
/// server.get("/", middleware! {
///     format!("{}", visits.fetch_add(1, Ordering::Relaxed))
/// });
///
/// server.listen("127.0.0.1:6767");
/// # }
/// ```
#[macro_export]
macro_rules! middleware {
    (|mut $res:ident| $($b:tt)+) => { _middleware_inner!($res, mut $res, $($b)+) };
    (|$res:ident| $($b:tt)+) => { _middleware_inner!($res, $res, $($b)+) };
    ($($b:tt)+) => { _middleware_inner!(_res, _res, $($b)+) };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _middleware_inner {
    ($res:ident, $res_binding:pat, $($b:tt)+) => {{
        use $crate::{MiddlewareResult,Responder, Response};

        #[inline(always)]
        fn restrict<'a, 'k, D, R: Responder<D>>(r: R, res: Response<'a, 'k, D>)
                -> MiddlewareResult<'a, 'k, D> {
            res.send(r)
        }

        // Inference fails due to thinking it's a (&Request, Response) with
        // different mutability requirements
        #[inline(always)]
        fn restrict_closure<F, D>(f: F) -> F
            where F: for<'a, 'k>
                        Fn(Response<'a, 'k, D>)
                            -> MiddlewareResult<'a, 'k, D> + Send + Sync { f }

        restrict_closure(move |$res_binding| {
            restrict(as_block!({$($b)+}), $res)
        })
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! as_block { ($b:block) => ( $b ) }

#[doc(hidden)]
#[macro_export]
macro_rules! as_pat { ($p:pat) => ( $p ) }
