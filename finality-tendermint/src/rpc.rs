use crate::Error;
use core::sync::atomic::{AtomicU64, Ordering};
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::{Future, StreamExt};
use message::*;
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::Arc;

impl RPCClient {
    async fn proposal(&self, msg: Proposal) -> Vec<ProposalReply> {
        unimplemented!()
    }

    async fn hello(&self, msg: String) -> Vec<String> {
        unimplemented!()
    }
}

// impl RPCServer {
//     async fn proposal(&self, msg: Proposal) -> ProposalReply {
//         unimplemented!()
//     }
//
//     async fn hello(&self, msg: String) -> String {
//         format!("Hi {} from {}", msg, "1")
//     }
// }

// enum Request {
//     Proposal(Proposal),
//     Prevote(Prevote),
//     Precommit(Precommit),
// }
//
// enum Response {
//     Proposal(ProposalReply),
//     Prevote(PrevoteReply),
//     Precommit(PrecommitReply),
// }

pub(crate) struct NewClient {
    client: RPCClient,
    /// A Future to be polled
    dispacth: Dispatch,
}

impl NewClient {
    pub(crate) fn new() -> Self {
        let (tx, rx) = unbounded();
        NewClient {
            client: RPCClient::new(tx),
            dispacth: Dispatch::new(rx),
        }
    }

    /// Spawn the Dispatch and return the Client.
    pub(crate) fn spawn(&self) -> RPCClient {
        tokio::spawn(self.dispacth.run());
        self.client
    }
}

pub(crate) struct RPCClient {
    /// A sender to Dispatch
    sender: UnboundedSender<DispatchRequest<Request, Response>>,
    /// Atomic counter for request id
    next_request_id: Arc<AtomicU64>,
}

impl RPCClient {
    pub(crate) fn new(tx: UnboundedSender<DispatchRequest<Request, Response>>) -> Self {
        RPCClient {
            sender: tx,
            next_request_id: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) async fn call(&mut self, req: Request) -> Vec<Response> {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = unbounded();
        self.sender.start_send(DispatchRequest {
            request: req,
            request_id: id,
            response_tx: tx,
        });

        let mut responses = Vec::new();

        // loop until the response is received or until timeout
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(1));

        tokio::pin!(timeout);

        loop {
            tokio::select! {
                res = rx.next() => {
                    if let Some(res) = res {
                        responses.push(res);
                    } else {
                        break;
                    }
                }
                _ = &mut timeout => {
                    break;
                }
            }
        }

        responses
    }
}

struct DispatchRequest<Req, Resp> {
    request: Req,
    request_id: u64,
    response_tx: UnboundedSender<Resp>,
}

struct Dispatch {
    /// Store Every handler register by client.
    ///
    /// By Sending Message to handler, the client will receive the reply msg from peers.
    handlers: BTreeMap<u64, UnboundedSender<String>>,
    /// A receiver to receive request from client.
    from_client: UnboundedReceiver<DispatchRequest<Request, Response>>,
    from_peers: UnboundedReceiver<Request>,
    to_peers: UnboundedSender<Request>,
}

impl Dispatch {
    pub(crate) fn new(
        from_client: UnboundedReceiver<DispatchRequest<Request, Response>>,
        from_peers: UnboundedReceiver<Request>,
        to_peers: UnboundedSender<Request>,
    ) -> Self {
        Dispatch {
            handlers: BTreeMap::new(),
            from_client,
            from_peers,
            to_peers,
        }
    }

    pub(crate) async fn run(self) {
        loop {
            tokio::select! {
                req = self.from_client.next() => {
                    if let Some(req) = req {
                        self.handle_request(req);
                    } else {
                        break;
                    }
                }
                res = self.from_peers.next() => {
                    if let Some(res) = res {
                        let resp = self.handle_request(res);

                    } else {
                        break;
                    }
                }
            }
        }
    }

    async fn handle_request(&self, res: Request) -> Response {
        unimplemented!()
    }
}

// struct RPCServer {
//     sender: UnboundedSender<String>,
//     receiver: UnboundedReceiver<String>,
// }
//
// impl RPCServer {
//     fn new() -> Self {
//         RPCServer {}
//     }
// }

// struct RPCPair {}
//
// impl RPCPair {}
