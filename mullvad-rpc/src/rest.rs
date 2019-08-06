use crate::HttpsConnectorWithSni;
use futures::{
    future::{self, Either},
    sync::{mpsc, oneshot},
    Future, Poll, Stream,
};
use hyper::{client::Client, Request, StatusCode, Uri};
use hyper_openssl::openssl::error::ErrorStack;
use std::path::Path;
use tokio_core::reactor::Handle;


pub type Result<T> = std::result::Result<T, Error>;

#[derive(derive_more::From, err_derive::Error, Debug)]
pub enum Error {
    /// When the http status code of the response is not 200 OK.
    #[error(display = "Http error. Status code {}", _0)]
    HttpError(StatusCode),

    /// An error occured in Hyper.
    #[error(display = "Error in HTTP client")]
    Hyper(#[error(cause)] hyper::Error),

    /// The string given was not a valid URI.
    #[error(display = "Not a valid URI")]
    Uri(#[error(cause)] hyper::error::UriError),

    /// Error in OpenSSL
    #[error(display = "Error in OpenSSL")]
    OpenSsl(#[error(cause)] ErrorStack),
}


pub type RequestSender = mpsc::UnboundedSender<(Request, oneshot::Sender<Result<Vec<u8>>>)>;
type RequestReceiver = mpsc::UnboundedReceiver<(Request, oneshot::Sender<Result<Vec<u8>>>)>;

struct SenderCloseWatcher<T>(oneshot::Sender<T>);

impl<T> Future for SenderCloseWatcher<T> {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll_cancel()
    }
}

impl<T> SenderCloseWatcher<T> {
    fn send(self, item: T) -> std::result::Result<(), T> {
        self.0.send(item)
    }
}

pub fn create_https_client<P: AsRef<Path>>(ca_path: P, handle: &Handle) -> Result<RequestSender> {
    let connector = HttpsConnectorWithSni::new(ca_path, handle)?;
    let client = Client::configure()
        .keep_alive(false)
        .connector(connector)
        .build(handle);

    let (request_tx, request_rx) = mpsc::unbounded();
    handle.spawn(create_request_processing_future(request_rx, client));
    Ok(request_tx)
}

fn create_request_processing_future<CC: hyper::client::Connect>(
    request_rx: RequestReceiver,
    client: Client<CC, hyper::Body>,
) -> Box<dyn Future<Item = (), Error = ()>> {
    let f = request_rx.for_each(move |(request, response_tx)| {
        log::trace!("Sending request to {}", request.uri());
        client
            .request(request)
            .from_err()
            .and_then(|response: hyper::Response| {
                if response.status() == hyper::StatusCode::Ok {
                    future::ok(response)
                } else {
                    future::err(Error::HttpError(response.status()).into())
                }
            })
            .and_then(|response: hyper::Response| response.body().concat2().from_err())
            .map(|response_chunk| response_chunk.to_vec())
            .select2(SenderCloseWatcher(response_tx))
            .then(move |response_or_cancellation| {
                let (response_result, response_tx) = match response_or_cancellation {
                    Ok(Either::A((response, response_tx))) => (Ok(response), response_tx),
                    Err(Either::A((error, response_tx))) => (Err(error), response_tx),

                    Ok(Either::B(((), _))) => {
                        log::warn!("HTTP request has been cancelled");
                        return Ok(());
                    }
                    Err(Either::B(((), _))) => unreachable!("SenderCloseWatcher is infallible"),
                };
                if response_tx.send(response_result).is_err() {
                    log::warn!("Unable to send response back to caller");
                }
                Ok(())
            })
    });
    Box::new(f) as Box<dyn Future<Item = (), Error = ()>>
}

pub fn create_get_request(uri: Uri) -> Request {
    Request::new(hyper::Method::Get, uri)
}
