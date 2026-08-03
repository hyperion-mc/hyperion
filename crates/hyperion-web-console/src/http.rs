//! The web server, and the whole of what it will answer.
//!
//! Five paths matched on a string. That is why this is hyper and not axum:
//! there is no routing to do, no extractors to derive, and hyper plus its two
//! helper crates are already in the lock through `reqwest`, so the console
//! costs no new dependency tree at all.
//!
//! # Authentication
//!
//! One shared secret, read from a file the server never writes and compared in
//! constant time. There are no accounts and no sessions on purpose: this is one
//! operator's window onto their own server, and everything an accounts system
//! would add is a second thing to get wrong.
//!
//! `POST` requests carry it as `Authorization: Bearer <token>`. The event
//! stream carries it as `?token=` instead, because `EventSource` cannot set a
//! header -- the browser API simply has no argument for it. So the token
//! reaches the server's own access log, if it has one, and the browser's
//! history. That is stated rather than hidden, and it is why the bind address
//! defaults to loopback: this is not a port to put on the internet.

use std::{
    convert::Infallible,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use http_body_util::{BodyExt, combinators::BoxBody};
use hyper::{
    Method, Request as HttpRequest, Response, StatusCode,
    body::{Body as HttpBody, Bytes, Frame, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{
    net::TcpListener,
    sync::{broadcast::error::RecvError, mpsc},
};

use crate::{
    Console,
    feed::{Line, Source},
    state::Request,
};

/// The page itself, compiled in.
///
/// One file, hand written, no build step. A console that needs `npm` before it
/// can be looked at is a console that stops working the first time nobody has
/// run `npm` recently.
const PAGE: &str = include_str!("../assets/console.html");

/// The largest body this will read.
///
/// A chat line is 256 bytes at the protocol level and a command is not much
/// more, so anything past this is not an operator typing.
const MAX_BODY: usize = 8 * 1024;

type Body = BoxBody<Bytes, Infallible>;

/// Serve on an already bound listener until the process ends.
///
/// Takes the listener rather than an address because the bind is the one
/// failure a caller can still do something about, and it has to happen before
/// the thread this runs on is spawned. See [`crate::spawn`].
pub async fn serve(console: Arc<Console>, listener: TcpListener) {
    match listener.local_addr() {
        Ok(address) => tracing::info!("console listening on http://{address}/"),
        Err(error) => tracing::warn!("console listening, but on what: {error}"),
    }

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // One connection failing to come up is not the listener
                // failing, so this must not end the loop.
                tracing::warn!("console accept failed: {error}");
                continue;
            }
        };

        let console = Arc::clone(&console);
        tokio::spawn(async move {
            let service = service_fn(move |request| route(Arc::clone(&console), request, peer));
            if let Err(error) = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                tracing::debug!("console connection from {peer} ended: {error}");
            }
        });
    }
}

async fn route(
    console: Arc<Console>,
    request: HttpRequest<Incoming>,
    peer: SocketAddr,
) -> Result<Response<Body>, Infallible> {
    // Owned, because the body is read by value further down and a borrow of
    // the request cannot outlive that.
    let (path, query) = {
        let (path, query) =
            split_query(request.uri().path_and_query().map_or("/", |pq| pq.as_str()));
        (path.to_owned(), query.to_owned())
    };

    Ok(match (request.method(), path.as_str()) {
        // The page is not behind the token. It has nothing in it: it is markup
        // that asks for a token and then goes and gets the data. Gating it
        // would mean an operator could not reach the box that asks for the
        // secret without already having sent the secret.
        (&Method::GET, "/") => html(PAGE),

        (&Method::GET, "/events") => {
            if console.authorises(query_token(&query).unwrap_or_default().as_bytes()) {
                events(&console)
            } else {
                refuse(peer, "/events")
            }
        }

        (&Method::GET, "/state") => {
            if authorised_header(&console, &request) {
                json(&console.snapshot())
            } else {
                refuse(peer, "/state")
            }
        }

        (&Method::POST, "/say" | "/command") => {
            if authorised_header(&console, &request) {
                match read_body(request).await {
                    Err(message) => text(StatusCode::PAYLOAD_TOO_LARGE, message),
                    Ok(body) if body.trim().is_empty() => {
                        text(StatusCode::BAD_REQUEST, "nothing to send")
                    }
                    Ok(body) => {
                        console.submit(if path == "/say" {
                            Request::Say(body.trim().to_owned())
                        } else {
                            Request::Command(body.trim().to_owned())
                        });
                        text(StatusCode::ACCEPTED, "queued")
                    }
                }
            } else {
                refuse(peer, &path)
            }
        }

        _ => text(StatusCode::NOT_FOUND, "no such path"),
    })
}

/// A refusal, said once per attempt at a real severity.
///
/// `warn` and not `debug`: somebody probing an admin port is the thing an
/// operator wants to find in a journal, and a line below `info` is invisible to
/// every query that filters on severity.
fn refuse(peer: SocketAddr, path: &str) -> Response<Body> {
    tracing::warn!("console rejected an unauthenticated {path} from {peer}");
    text(StatusCode::UNAUTHORIZED, "a bearer token is required")
}

fn authorised_header(console: &Console, request: &HttpRequest<Incoming>) -> bool {
    let offered = request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    console.authorises(offered.as_bytes())
}

/// The live stream: the backlog first, then everything as it happens.
///
/// The backlog is taken *after* subscribing, so a line pushed between the two
/// can be sent twice but can never be lost. Duplicates the page can see and
/// drop by sequence number; a hole it cannot.
fn events(console: &Console) -> Response<Body> {
    let mut live = console.feed.subscribe();
    let backlog = console.feed.backlog();

    let (sender, receiver) = mpsc::channel::<Bytes>(64);

    tokio::spawn(async move {
        for line in backlog {
            if sender.send(sse(&line)).await.is_err() {
                return;
            }
        }

        loop {
            let chunk = match live.recv().await {
                Ok(entry) => sse(&entry),
                // The browser fell behind far enough to lose lines. Saying so
                // is the point: a gap the page does not know about is a page
                // that has quietly stopped being the truth.
                Err(RecvError::Lagged(missed)) => Bytes::from(format!(
                    "data: {}\n\n",
                    Line {
                        seq: 0,
                        at: 0,
                        source: Source::System,
                        text: format!("\u{a7}8[{missed} lines dropped: this browser fell behind]"),
                    }
                    .to_json()
                )),
                Err(RecvError::Closed) => return,
            };

            if sender.send(chunk).await.is_err() {
                return;
            }
        }
    });

    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-store")
        // Any reverse proxy in front of this must not buffer, or a live stream
        // arrives in silent bursts and looks like a hang.
        .header("x-accel-buffering", "no")
        .body(BodyExt::boxed(EventStream { receiver }))
        .expect("a response with only static headers cannot fail to build")
}

/// The SSE body: whatever the sender above puts on the channel.
///
/// Hand written rather than reached for through a stream adapter crate,
/// because the whole of it is "forward one channel" and the alternative is a
/// dependency for eleven lines.
struct EventStream {
    receiver: mpsc::Receiver<Bytes>,
}

impl HttpBody for EventStream {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.receiver
            .poll_recv(context)
            .map(|chunk| chunk.map(|bytes| Ok(Frame::data(bytes))))
    }
}

fn sse(line: &Line) -> Bytes {
    Bytes::from(format!("data: {}\n\n", line.to_json()))
}

async fn read_body(request: HttpRequest<Incoming>) -> Result<String, &'static str> {
    let collected = request
        .into_body()
        .collect()
        .await
        .map_err(|_unused| "could not read the body")?;
    let bytes = collected.to_bytes();
    if bytes.len() > MAX_BODY {
        return Err("body too large");
    }
    String::from_utf8(bytes.to_vec()).map_err(|_unused| "body was not utf-8")
}

fn split_query(path_and_query: &str) -> (&str, &str) {
    path_and_query
        .split_once('?')
        .map_or((path_and_query, ""), |(path, query)| (path, query))
}

fn query_token(query: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token=").map(percent_decode))
}

/// Enough of percent decoding for a token.
///
/// `%XX` and nothing else. A `+` decodes to a literal plus rather than to a
/// space: the page sends the token through `encodeURIComponent`, which spells
/// a space `%20` and leaves `+` untouched, so the form-encoding rule can never
/// recover a space anyone sent and can only corrupt a token that contains a
/// plus. Standard base64 emits `+` and `/`, so the obvious way to generate a
/// token produces exactly the value that rule breaks, and it breaks it as a
/// 401 with nothing in the log to say why. That is not hypothetical: it is
/// what `tools/console-check.py` hit on its first run against a live server.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_unused) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn html(body: &'static str) -> Response<Body> {
    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(full(Bytes::from_static(body.as_bytes())))
        .expect("a response with only static headers cannot fail to build")
}

fn json(body: &str) -> Response<Body> {
    Response::builder()
        .header("content-type", "application/json")
        .header("cache-control", "no-store")
        .body(full(Bytes::from(body.to_owned())))
        .expect("a response with only static headers cannot fail to build")
}

fn text(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(full(Bytes::from(body.to_owned())))
        .expect("a response with only static headers cannot fail to build")
}

fn full(bytes: Bytes) -> Body {
    BodyExt::boxed(http_body_util::Full::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::{percent_decode, query_token};

    /// What `head -c 24 /dev/urandom | base64` produces: standard base64,
    /// which uses `+` and `/`. The console reads its token from a file an
    /// operator generated, so this is the ordinary case rather than an edge.
    const BASE64_TOKEN: &str = "z/qKP4ZL2WfCDIGq8Sh0+dt1i96XQRq1";

    /// The bug this pins. Treating `+` as a space -- the form-encoding rule --
    /// turned a token a browser had sent correctly into a 401 that said
    /// nothing about why.
    #[test]
    fn a_plus_is_a_plus_and_not_a_space() {
        assert_eq!(percent_decode("a+b"), "a+b");
        assert_eq!(percent_decode(BASE64_TOKEN), BASE64_TOKEN);
    }

    /// What the page actually sends: `encodeURIComponent` escapes both of
    /// base64's awkward characters, and the decode has to give the token back
    /// unchanged or the page cannot open its own event stream.
    #[test]
    fn the_encoding_the_page_uses_round_trips() {
        let encoded = "z%2FqKP4ZL2WfCDIGq8Sh0%2Bdt1i96XQRq1";
        assert_eq!(percent_decode(encoded), BASE64_TOKEN);
        assert_eq!(
            query_token(&format!("token={encoded}")).as_deref(),
            Some(BASE64_TOKEN)
        );
    }

    /// A space still arrives when somebody really sent one: dropping the `+`
    /// rule costs nothing, because `encodeURIComponent(' ')` is `%20`.
    #[test]
    fn a_percent_twenty_is_still_a_space() {
        assert_eq!(percent_decode("a%20b"), "a b");
    }

    /// A `%` that does not begin an escape is a `%`, rather than eating the
    /// bytes after it and silently shortening the token.
    #[test]
    fn a_stray_percent_is_literal() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
    }

    #[test]
    fn a_token_is_found_among_other_parameters() {
        assert_eq!(query_token("since=4&token=abc").as_deref(), Some("abc"));
        assert_eq!(query_token("since=4"), None);
    }
}
