use std::str::FromStr;
use std::sync::Arc;
use std::{fmt, str};

use async_trait::async_trait;
use http::{Request, Response, StatusCode};
use serde::Deserialize;

pub use http;
pub use mendes_macros::{dispatch, handler};

#[cfg(feature = "cookies")]
pub mod cookies;

mod form;
#[cfg(feature = "uploads")]
mod multipart;

pub mod forms {
    pub use super::form::*;
    #[cfg(feature = "uploads")]
    pub use super::multipart::{from_form_data, File};
}

#[cfg(feature = "hyper")]
pub mod hyper {
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::sync::Arc;

    use futures_util::future::FutureExt;
    use http::{header::LOCATION, Response, StatusCode};
    use hyper::service::{make_service_fn, service_fn};
    use hyper::{Body, Server};

    use super::{Application, Context};

    pub async fn run<A>(addr: &SocketAddr, app: A) -> Result<(), hyper::Error>
    where
        A: Application<RequestBody = Body, ResponseBody = Body> + Send + Sync + 'static,
    {
        let app = Arc::new(app);
        Server::bind(addr)
            .serve(make_service_fn(move |_| {
                let app = app.clone();
                async {
                    Ok::<_, Infallible>(service_fn(move |req| {
                        let cx = Context::new(app.clone(), req);
                        A::handle(cx).map(Ok::<_, Infallible>)
                    }))
                }
            }))
            .await
    }

    pub fn redirect(status: StatusCode, path: &str) -> Response<Body> {
        http::Response::builder()
            .status(status)
            .header(LOCATION, path)
            .body(Body::empty())
            .unwrap()
    }
}

#[async_trait]
pub trait Application: Sized {
    type RequestBody;
    type ResponseBody;
    type Error: From<ClientError>;

    async fn handle(cx: Context<Self>) -> Response<Self::ResponseBody>;

    fn error(&self, error: Self::Error) -> Response<Self::ResponseBody>;
}

pub struct Context<A>
where
    A: Application,
{
    app: Arc<A>,
    req: http::request::Parts,
    body: Option<A::RequestBody>,
    path: PathState,
}

impl<A> Context<A>
where
    A: Application,
{
    pub fn new(app: Arc<A>, req: Request<A::RequestBody>) -> Context<A> {
        let path = PathState::new(req.uri().path());
        let (req, body) = req.into_parts();
        Context {
            app,
            req,
            body: Some(body),
            path,
        }
    }

    pub fn next(&mut self) -> Option<&str> {
        self.path.next(&self.req.uri.path())
    }

    pub fn rest(&mut self) -> &str {
        self.path.rest(&self.req.uri.path())
    }

    pub fn take_body(&mut self) -> Option<A::RequestBody> {
        self.body.take()
    }

    pub fn rewind(mut self) -> Self {
        self.path.rewind();
        self
    }

    pub fn app(&self) -> &Arc<A> {
        &self.app
    }

    pub fn req(&self) -> &http::request::Parts {
        &self.req
    }

    pub fn method(&self) -> &http::Method {
        &self.req.method
    }

    pub fn uri(&self) -> &http::uri::Uri {
        &self.req.uri
    }

    pub fn headers(&self) -> &http::HeaderMap {
        &self.req.headers
    }
}

pub trait FromContext<'a>: Sized {
    fn from_context<A: Application>(cx: &'a mut Context<A>) -> Result<Self, A::Error>;
}

impl<'a> FromContext<'a> for &'a http::request::Parts {
    fn from_context<A: Application>(cx: &'a mut Context<A>) -> Result<Self, A::Error> {
        Ok(&cx.req)
    }
}

impl<'a> FromContext<'a> for Option<&'a str> {
    fn from_context<A: Application>(cx: &'a mut Context<A>) -> Result<Self, A::Error> {
        Ok(cx.next())
    }
}

impl<'a> FromContext<'a> for &'a str {
    fn from_context<A: Application>(cx: &'a mut Context<A>) -> Result<Self, A::Error> {
        cx.next().ok_or_else(|| ClientError::NotFound.into())
    }
}

impl<'a> FromContext<'a> for usize {
    fn from_context<A: Application>(cx: &'a mut Context<A>) -> Result<Self, A::Error> {
        let s = cx.next().ok_or(ClientError::NotFound)?;
        usize::from_str(s).map_err(|_| ClientError::NotFound.into())
    }
}

impl<'a> FromContext<'a> for i32 {
    fn from_context<A: Application>(cx: &'a mut Context<A>) -> Result<Self, A::Error> {
        let s = cx.next().ok_or(ClientError::NotFound)?;
        i32::from_str(s).map_err(|_| ClientError::NotFound.into())
    }
}

struct PathState {
    prev: Option<usize>,
    next: Option<usize>,
}

impl PathState {
    fn new(path: &str) -> Self {
        let next = if path.is_empty() || path == "/" {
            None
        } else if path.find('/') == Some(0) {
            Some(1)
        } else {
            Some(0)
        };
        Self { prev: None, next }
    }

    fn next<'r>(&mut self, path: &'r str) -> Option<&'r str> {
        let start = match self.next.as_ref() {
            Some(v) => *v,
            None => return None,
        };

        let path = &path[start..];
        if path.is_empty() {
            self.prev = self.next.take();
            return None;
        }

        match path.find('/') {
            Some(end) => {
                self.prev = self.next.replace(start + end + 1);
                Some(&path[..end])
            }
            None => {
                self.prev = self.next.take();
                Some(path)
            }
        }
    }

    fn rest<'r>(&mut self, path: &'r str) -> &'r str {
        let start = match self.next.take() {
            Some(v) => v,
            None => return "",
        };

        self.prev = Some(start);
        &path[start..]
    }

    fn rewind(&mut self) {
        self.next = self.prev.take();
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ClientError {
    BadRequest,
    Unauthorized,
    PaymentRequired,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    NotAcceptable,
    ProxyAuthenticationRequired,
    RequestTimeout,
    Conflict,
    Gone,
    LengthRequired,
    PreconditionFailed,
    PayloadTooLarge,
    RequestUriTooLong,
    UnsupportedMediaType,
    RequestedRangeNotSatisfiable,
    ExpectationFailed,
    MisdirectedRequest,
    UnprocessableEntity,
    Locked,
    FailedDependency,
    UpgradeRequired,
    PreconditionRequired,
    TooManyRequests,
    RequestHeaderFieldsTooLarge,
    UnavailableForLegalReasons,
}

impl From<ClientError> for StatusCode {
    fn from(e: ClientError) -> StatusCode {
        use ClientError::*;
        match e {
            BadRequest => StatusCode::BAD_REQUEST,
            Unauthorized => StatusCode::UNAUTHORIZED,
            PaymentRequired => StatusCode::PAYMENT_REQUIRED,
            Forbidden => StatusCode::FORBIDDEN,
            NotFound => StatusCode::NOT_FOUND,
            MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            NotAcceptable => StatusCode::NOT_ACCEPTABLE,
            ProxyAuthenticationRequired => StatusCode::PROXY_AUTHENTICATION_REQUIRED,
            RequestTimeout => StatusCode::REQUEST_TIMEOUT,
            Conflict => StatusCode::CONFLICT,
            Gone => StatusCode::GONE,
            LengthRequired => StatusCode::LENGTH_REQUIRED,
            PreconditionFailed => StatusCode::PRECONDITION_FAILED,
            PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            RequestUriTooLong => StatusCode::URI_TOO_LONG,
            UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            RequestedRangeNotSatisfiable => StatusCode::RANGE_NOT_SATISFIABLE,
            ExpectationFailed => StatusCode::EXPECTATION_FAILED,
            MisdirectedRequest => StatusCode::MISDIRECTED_REQUEST,
            UnprocessableEntity => StatusCode::UNPROCESSABLE_ENTITY,
            Locked => StatusCode::LOCKED,
            FailedDependency => StatusCode::FAILED_DEPENDENCY,
            UpgradeRequired => StatusCode::UPGRADE_REQUIRED,
            PreconditionRequired => StatusCode::PRECONDITION_REQUIRED,
            TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            RequestHeaderFieldsTooLarge => StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            UnavailableForLegalReasons => StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
        }
    }
}

#[allow(unused_variables, unreachable_code, dead_code)]
pub fn from_body_bytes<'de, T: 'de + Deserialize<'de>>(
    headers: &http::HeaderMap,
    bytes: &'de [u8],
) -> Result<T, ClientError> {
    let inner = match headers.get("content-type") {
        #[cfg(feature = "serde_urlencoded")]
        Some(t) if t == "application/x-www-form-urlencoded" => {
            serde_urlencoded::from_bytes::<T>(bytes)
                .map_err(|_| ClientError::UnprocessableEntity)?
        }
        #[cfg(feature = "serde_json")]
        Some(t) if t == "application/json" => {
            serde_json::from_slice::<T>(bytes).map_err(|_| ClientError::UnprocessableEntity)?
        }
        #[cfg(feature = "uploads")]
        Some(t) if t.as_bytes().starts_with(b"multipart/form-data") => {
            forms::from_form_data::<T>(headers, bytes)
                .map_err(|_| ClientError::UnprocessableEntity)?
        }
        None => return Err(ClientError::BadRequest),
        _ => return Err(ClientError::UnsupportedMediaType),
    };

    Ok(inner)
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "{}", StatusCode::from(*self))
    }
}

pub mod types {
    pub const HTML: &str = "text/html";
    pub const JSON: &str = "application/json";
}
