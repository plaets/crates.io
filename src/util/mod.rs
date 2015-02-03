use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::old_io::MemReader;
use std::old_io::process::{ProcessOutput, Command};
use std::str;
use std::sync::Arc;

use rustc_serialize::{json, Encodable};
use rustc_serialize::json::Json;
use url;

use conduit::{Request, Response, Handler};
use conduit_router::{RouteBuilder, RequestParams};
use db::RequestTransaction;
use self::errors::NotFound;

pub use self::errors::{CargoError, CargoResult, internal, human, internal_error};
pub use self::errors::{ChainError, std_error};
pub use self::hasher::{HashingReader};
pub use self::head::Head;
pub use self::io::LimitErrorReader;
pub use self::lazy_cell::LazyCell;
pub use self::request_proxy::RequestProxy;

pub mod errors;
mod hasher;
mod head;
mod io;
mod lazy_cell;
mod request_proxy;

pub trait RequestUtils {
    fn redirect(self, url: String) -> Response;

    fn json<T: Encodable>(self, t: &T) -> Response;
    fn query(self) -> HashMap<String, String>;
    fn wants_json(self) -> bool;
    fn pagination(self, default: usize, max: usize) -> CargoResult<(i64, i64)>;
}

pub fn json_response<T: Encodable>(t: &T) -> Response {
    let s = json::encode(t).unwrap();
    let json = fixup(s.parse().unwrap()).to_string();
    let mut headers = HashMap::new();
    headers.insert("Content-Type".to_string(),
                   vec!["application/json; charset=utf-8".to_string()]);
    headers.insert("Content-Length".to_string(), vec![json.len().to_string()]);
    return Response {
        status: (200, "OK"),
        headers: headers,
        body: Box::new(MemReader::new(json.into_bytes())),
    };

    fn fixup(json: Json) -> Json {
        match json {
            Json::Object(object) => {
                Json::Object(object.into_iter().map(|(k, v)| {
                    let k = if k.as_slice() == "krate" {
                        "crate".to_string()
                    } else {
                        k
                    };
                    (k, fixup(v))
                }).collect())
            }
            Json::Array(list) => {
                Json::Array(list.into_iter().map(fixup).collect())
            }
            j => j,
        }
    }
}


impl<'a> RequestUtils for &'a (Request + 'a) {
    fn json<T: Encodable>(self, t: &T) -> Response {
        json_response(t)
    }

    fn query(self) -> HashMap<String, String> {
        url::form_urlencoded::parse(self.query_string().unwrap_or("")
                                        .as_bytes())
            .into_iter().collect()
    }

    fn redirect(self, url: String) -> Response {
        let mut headers = HashMap::new();
        headers.insert("Location".to_string(), vec![url.to_string()]);
        Response {
            status: (302, "Found"),
            headers: headers,
            body: Box::new(MemReader::new(Vec::new())),
        }
    }

    fn wants_json(self) -> bool {
        let content = self.headers().find("Accept").unwrap_or(Vec::new());
        content.iter().any(|s| s.contains("json"))
    }

    fn pagination(self, default: usize, max: usize) -> CargoResult<(i64, i64)> {
        let query = self.query();
        let page = query.get("page").and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(1);
        let limit = query.get("per_page").and_then(|s| s.parse::<usize>().ok())
                         .unwrap_or(default);
        if limit > max {
            return Err(human(format!("cannot request more than {} items", max)))
        }
        Ok((((page - 1) * limit) as i64, limit as i64))
    }
}

pub struct C(pub fn(&mut Request) -> CargoResult<Response>);

impl Handler for C {
    fn call(&self, req: &mut Request) -> Result<Response, Box<Error+Send>> {
        let C(f) = *self;
        match f(req) {
            Ok(resp) => { req.commit(); Ok(resp) }
            Err(e) => {
                match e.response() {
                    Some(response) => Ok(response),
                    None => Err(std_error(e))
                }
            }
        }
    }
}

pub struct R<H>(pub Arc<H>);

impl<H: Handler> Handler for R<H> {
    fn call(&self, req: &mut Request) -> Result<Response, Box<Error+Send>> {
        let path = req.params()["path"].to_string();
        let R(ref sub_router) = *self;
        sub_router.call(&mut RequestProxy {
            other: req,
            path: Some(path.as_slice()),
            method: None,
        })
    }
}

pub struct R404(pub RouteBuilder);

impl Handler for R404 {
    fn call(&self, req: &mut Request) -> Result<Response, Box<Error+Send>> {
        let R404(ref router) = *self;
        match router.recognize(&req.method(), req.path()) {
            Ok(m) => {
                req.mut_extensions().insert(m.params.clone());
                m.handler.call(req)
            }
            Err(..) => Ok(NotFound.response().unwrap()),
        }
    }
}

pub fn exec(cmd: &Command) -> CargoResult<ProcessOutput> {
    let output = try!(cmd.output().chain_error(|| {
        internal(format!("failed to run command `{:?}`", cmd))
    }));
    if !output.status.success() {
        let mut desc = String::new();
        if output.output.len() != 0 {
            desc.push_str("--- stdout\n");
            desc.push_str(str::from_utf8(output.output.as_slice()).unwrap());
        }
        if output.error.len() != 0 {
            desc.push_str("--- stderr\n");
            desc.push_str(str::from_utf8(output.error.as_slice()).unwrap());
        }
        Err(internal_error(format!("failed to run command `{:?}`", cmd), desc))
    } else {
        Ok(output)
    }
}

pub struct CommaSep<'a, T: 'a>(pub &'a [T]);

impl<'a, T: fmt::Display> fmt::Display for CommaSep<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, t) in self.0.iter().enumerate() {
            if i != 0 { try!(write!(f, ", ")); }
            try!(write!(f, "{}", t));
        }
        Ok(())
    }

}
