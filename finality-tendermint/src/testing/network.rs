use crate::{
    messages::{Callback, GlobalMessageIn, GlobalMessageOut, Message, SignedMessage},
    Error,
};

use tokio::sync::Notify;
use tracing::instrument;

use super::*;
use core::{
    pin::Pin,
    task::{Context, Poll, Waker},
};
use futures::{
    channel::mpsc::{self, UnboundedReceiver, UnboundedSender},
    Future, Sink, SinkExt, Stream, StreamExt,
};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc};

/// p2p network data for a round.
///
/// Every node can send `Message` to the network, then it will be
/// wrapped in `SignedMessage` and broadcast to all other nodes.
struct BroadcastNetwork<M> {
    /// Receiver from every peer on the network.
    receiver: UnboundedReceiver<(Id, M)>,
    /// Raw sender to give a new node join the network.
    raw_sender: UnboundedSender<(Id, M)>,
    /// Peer's sender to send messages to.
    senders: Vec<(Id, UnboundedSender<M>)>,
    /// Broadcast history.
    history: Vec<M>,
    /// A validator hook is a hook to decide whether a message should be broadcast.
    /// By default, all messages are broadcast.
    validator_hook: Option<Box<dyn Fn(&M) -> () + Send + Sync>>,
    /// A routing table that decide whether a message should be sent to a peer.
    rule: Arc<Mutex<RoutingRule>>,
}

impl<M: Clone + std::fmt::Debug> BroadcastNetwork<M> {
    fn new(rule: Arc<Mutex<RoutingRule>>) -> Self {
        let (tx, rx) = mpsc::unbounded();
        BroadcastNetwork {
            receiver: rx,
            raw_sender: tx,
            senders: Vec::new(),
            history: Vec::new(),
            validator_hook: None,
            rule,
        }
    }

    fn register_validator_hook(&mut self, hook: Box<dyn Fn(&M) -> () + Send + Sync>) {
        self.validator_hook = Some(hook);
    }

    /// Add a node to the network for a round.
    fn add_node<N, F: Fn(N) -> M>(
        &mut self,
        id: Id,
        f: F,
    ) -> (
        impl Stream<Item = Result<M, Error>>,
        impl Sink<N, Error = Error>,
    ) {
        tracing::trace!("BroadcastNetwork::add_node");
        // Channel from Network to the new node.
        let (tx, rx) = mpsc::unbounded();
        let messages_out = self
            .raw_sender
            .clone()
            .sink_map_err(|e| panic!("Error sending message: {:?}", e))
            .with(move |message| std::future::ready(Ok((id, f(message)))));

        // get history to the node.
        // for prior_message in self.history.iter().cloned() {
        // 	let _ = tx.unbounded_send(prior_message);
        // }

        // tracing::trace!("add_node: tx.isclosed? {}", tx.is_closed());
        self.senders.push((id, tx));

        tracing::trace!("BroadcastNetwork::add_node end.");
        (rx.map(Ok), messages_out)
    }

    // Do routing work
    fn route(&mut self, cx: &mut Context) -> Poll<()> {
        loop {
            // Receive item from receiver
            match Pin::new(&mut self.receiver).poll_next(cx) {
                // While have message
                Poll::Ready(Some((ref from, msg))) => {
                    self.history.push(msg.clone());

                    // Validate message.
                    if let Some(hook) = &self.validator_hook {
                        hook(&msg);
                    }

                    tracing::trace!("    msg {:?}", msg);
                    // Broadcast to all peers including itself.
                    for (to, sender) in &self.senders {
                        if self.rule.lock().valid_route(from, to) {
                            let _res = sender.unbounded_send(msg.clone());
                        }
                        // tracing::trace!("route: tx.isclosed? {}", sender.is_closed());
                        // tracing::trace!("res: {:?}", res);
                    }
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(()),
            }
        }
    }

    /// Broadcast a message to all peers.
    pub fn send_message(&self, message: M) {
        // TODO: id: `0` is not used.
        let _ = self.raw_sender.unbounded_send((0, message));
    }
}

/// Network for a round.
type RoundNetwork = BroadcastNetwork<SignedMessage<BlockNumber, Hash, Signature, Id>>;
/// Global Network.
type GlobalMessageNetwork = BroadcastNetwork<GlobalMessageIn<Hash, BlockNumber, Signature, Id>>;

pub(crate) fn make_network() -> (Network, NetworkRouting) {
    let rule = Arc::new(Mutex::new(RoutingRule::new()));

    let rounds = Arc::new(Mutex::new(HashMap::new()));
    let global = Arc::new(Mutex::new(GlobalMessageNetwork::new(rule.clone())));

    let notify = Arc::new(Mutex::new(None));
    (
        Network {
            rounds: rounds.clone(),
            global: global.clone(),
            rule: rule.clone(),
            notify: notify.clone(),
        },
        NetworkRouting {
            rounds,
            global,
            rule,
            notify,
        },
    )
}

/// State of a voter.
/// Used to pass to the [`Rule`] hook.
#[derive(Default, Copy, Clone)]
pub(crate) struct VoterState {
    pub(crate) last_finalized: BlockNumber,
    pub(crate) view_number: u64,
}

impl VoterState {
    pub fn new() -> Self {
        Self {
            last_finalized: 0,
            view_number: 0,
        }
    }
}

/// Rule type for routing table.
type Rule = Box<dyn Send + Fn(&Id, &VoterState, &Id, &VoterState) -> bool>;

/// Routing table.
#[derive(Default)]
pub(crate) struct RoutingRule {
    /// All peers in the network, same as VoterSet.
    nodes: Vec<Id>,
    /// Track all peers' state.
    state_tracker: HashMap<Id, VoterState>,
    /// Rule for routing.
    /// By default, all messages are broadcast.
    rules: Vec<Rule>,
}

impl RoutingRule {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            state_tracker: HashMap::new(),
            rules: Vec::new(),
        }
    }

    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Update peer's state.
    pub fn update_state(&mut self, id: Id, state: VoterState) {
        self.state_tracker.insert(id, state);
    }

    /// Check if a message is valid to route.
    pub fn valid_route(&mut self, from: &Id, to: &Id) -> bool {
        match (self.state_tracker.get(from), self.state_tracker.get(to)) {
            (Some(_), None) => {
                self.state_tracker.insert(to.clone(), VoterState::new());
            }
            (None, Some(_)) => {
                self.state_tracker.insert(from.clone(), VoterState::new());
            }
            (None, None) => {
                self.state_tracker.insert(from.clone(), VoterState::new());
                self.state_tracker.insert(to.clone(), VoterState::new());
            }
            _ => {}
        };

        let from_state = self.state_tracker.get(from).unwrap();
        let to_state = self.state_tracker.get(to).unwrap();
        self.rules
            .iter()
            .map(|rule| rule(from, from_state, to, to_state))
            .all(|v| v)
    }

    /// A preset rule to isolate a peer from others.
    pub fn isolate(&mut self, node: Id) {
        let _isolate =
            move |from: &Id, _from_state: &VoterState, to: &Id, _to_state: &VoterState| {
                !(from == &node || to == &node)
            };
        self.add_rule(Box::new(_isolate));
    }

    /// A preset rule to isolate a peer from others after required block height.
    pub fn isolate_after(&mut self, node: Id, after: BlockNumber) {
        let _isolate_after =
            move |from: &Id, from_state: &VoterState, to: &Id, to_state: &VoterState| {
                !((from == &node && from_state.last_finalized >= after)
                    || (to == &node && to_state.last_finalized >= after))
            };
        self.add_rule(Box::new(_isolate_after));
    }
}

/// the network routing task.
pub struct NetworkRouting {
    /// Key: view number, Value: RoundNetwork
    rounds: Arc<Mutex<HashMap<u64, RoundNetwork>>>,
    /// Global message network.
    global: Arc<Mutex<GlobalMessageNetwork>>,
    /// Routing rule.
    pub(crate) rule: Arc<Mutex<RoutingRule>>,
    notify: Arc<Mutex<Option<Waker>>>,
}

impl NetworkRouting {
    pub fn register_global_validator_hook(
        &mut self,
        hook: Box<dyn Fn(&GlobalMessageIn<Hash, BlockNumber, Signature, Id>) -> () + Sync + Send>,
    ) {
        self.global.lock().register_validator_hook(hook);
    }
}

impl Future for NetworkRouting {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.notify.lock().insert(cx.waker().clone());

        tracing::trace!("NetworkRouting::poll start.");
        let mut rounds = self.rounds.lock();
        // Retain all round that not finished
        rounds.retain(|view, round_network| {
            tracing::trace!("  view: {view} poll");
            let ret = match round_network.route(cx) {
                Poll::Ready(()) => {
                    tracing::trace!(
                        "    view: {view}, round_network.route: finished with history: {}",
                        round_network.history.len()
                    );

                    false
                }
                Poll::Pending => true,
            };

            tracing::trace!("  view: {view} poll end");

            ret
        });

        let mut global = self.global.lock();
        let _ = global.route(cx);

        tracing::trace!("NetworkRouting::poll end.");

        // Nerver stop.
        Poll::Pending
    }
}

/// A test network. Instantiate this with `make_network`,
#[derive(Clone)]
pub struct Network {
    rounds: Arc<Mutex<HashMap<u64, RoundNetwork>>>,
    global: Arc<Mutex<GlobalMessageNetwork>>,
    pub(crate) rule: Arc<Mutex<RoutingRule>>,
    notify: Arc<Mutex<Option<Waker>>>,
}

impl Network {
    /// Initialize a round network.
    #[instrument(level = "trace", skip(self))]
    pub fn make_round_comms(
        &self,
        view_number: u64,
        node_id: Id,
    ) -> (
        impl Stream<Item = Result<SignedMessage<BlockNumber, Hash, Signature, Id>, Error>>,
        impl Sink<Message<BlockNumber, Hash>, Error = Error>,
    ) {
        tracing::trace!(
            "make_round_comms, view_number: {}, node_id: {}",
            view_number,
            node_id
        );
        let mut rounds = self.rounds.lock();
        let round_comm = rounds
            .entry(view_number)
            .or_insert_with(|| RoundNetwork::new(self.rule.clone()))
            .add_node(node_id, move |message| SignedMessage {
                message,
                signature: node_id,
                id: node_id,
            });

        for (key, value) in rounds.iter() {
            tracing::trace!(
                "  round_comms: {}, senders.len:{:?}",
                key,
                value.senders.len()
            );
        }

        tracing::trace!("make_round_comms end");

        // Notify the network routing task.
        self.notify
            .as_ref()
            .lock()
            .as_ref()
            .map(|waker| waker.wake_by_ref());

        round_comm
    }

    /// Initialize the global network.
    pub fn make_global_comms(
        &self,
        id: Id,
    ) -> (
        impl Stream<Item = Result<GlobalMessageIn<Hash, BlockNumber, Signature, Id>, Error>>,
        impl Sink<GlobalMessageOut<Hash, BlockNumber, Signature, Id>, Error = Error>,
    ) {
        tracing::trace!("make_global_comms");
        let mut global = self.global.lock();
        let f: fn(GlobalMessageOut<_, _, _, _>) -> GlobalMessageIn<_, _, _, _> = |msg| match msg {
            GlobalMessageOut::Commit(view, commit) => {
                GlobalMessageIn::Commit(view, commit.into(), Callback::Blank)
            }

            GlobalMessageOut::Empty => GlobalMessageIn::Empty,
        };
        let global_comm = global.add_node(id, f);

        global_comm
    }

    /// Send a message to all nodes.
    pub fn send_message(&self, message: GlobalMessageIn<Hash, BlockNumber, Signature, Id>) {
        self.global.lock().send_message(message);
    }
}
