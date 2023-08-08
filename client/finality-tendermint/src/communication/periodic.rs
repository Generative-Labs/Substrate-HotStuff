//! Periodic rebroadcast of neighbor packets.

use futures::{future::FutureExt as _, prelude::*, ready, stream::Stream};
use futures_timer::Delay;
use log::debug;
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedReceiver, TracingUnboundedSender};
use std::{
	pin::Pin,
	task::{Context, Poll},
	time::Duration,
};

use super::gossip::{GossipMessage, NeighborPacket};
use sc_network::PeerId;
use sp_runtime::traits::{Block as BlockT, NumberFor};

// How often to rebroadcast, in cases where no new packets are created.
const REBROADCAST_AFTER: Duration = Duration::from_secs(2 * 60);

/// A sender used to send neighbor packets to a background job.
#[derive(Clone)]
pub(super) struct NeighborPacketSender<B: BlockT>(
	TracingUnboundedSender<(Vec<PeerId>, NeighborPacket<NumberFor<B>>)>,
);

impl<B: BlockT> NeighborPacketSender<B> {
	/// Send a neighbor packet for the background worker to gossip to peers.
	pub fn send(
		&self,
		who: Vec<sc_network::PeerId>,
		neighbor_packet: NeighborPacket<NumberFor<B>>,
	) {
		if let Err(err) = self.0.unbounded_send((who, neighbor_packet)) {
			debug!(target: "afp", "Failed to send neighbor packet: {:?}", err);
		}
	}
}

/// NeighborPacketWorker is listening on a channel for new neighbor packets being produced by
/// components within `finality-grandpa` and forwards those packets to the underlying
/// `NetworkEngine` through the `NetworkBridge` that it is being polled by (see `Stream`
/// implementation).
/// NOTE: Periodically it sends out the last packet in cases where no new one arrive.
pub(super) struct NeighborPacketWorker<B: BlockT> {
	last: Option<(Vec<PeerId>, NeighborPacket<NumberFor<B>>)>,
	delay: Delay,
	rx: TracingUnboundedReceiver<(Vec<PeerId>, NeighborPacket<NumberFor<B>>)>,
}
impl<B: BlockT> Unpin for NeighborPacketWorker<B> {}

impl<B: BlockT> NeighborPacketWorker<B> {
	pub(super) fn new() -> (Self, NeighborPacketSender<B>) {
		let (tx, rx) = tracing_unbounded::<(Vec<PeerId>, NeighborPacket<NumberFor<B>>)>(
			"mpsc_grandpa_neighbor_packet_worker",
			10_000,
		);
		let delay = Delay::new(REBROADCAST_AFTER);

		(NeighborPacketWorker { last: None, delay, rx }, NeighborPacketSender(tx))
	}
}

impl<B: BlockT> Stream for NeighborPacketWorker<B> {
	type Item = (Vec<PeerId>, GossipMessage<B>);

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
		let this = &mut *self;
		match this.rx.poll_next_unpin(cx) {
			Poll::Ready(None) => return Poll::Ready(None),
			Poll::Ready(Some((to, packet))) => {
				this.delay.reset(REBROADCAST_AFTER);
				this.last = Some((to.clone(), packet.clone()));

				return Poll::Ready(Some((to, GossipMessage::<B>::from(packet))))
			},
			// Don't return yet, maybe the timer fired.
			Poll::Pending => {},
		}

		ready!(this.delay.poll_unpin(cx));

		// Getting this far here implies that the timer fired.

		this.delay.reset(REBROADCAST_AFTER);

		// Make sure the underlying task is scheduled for wake-up.
		//
		// NOTE: In case poll_unpin is called after the resetted delay fires again, this
		// will drop one tick. Deemed as very unlikely and also not critical.
		while let Poll::Ready(()) = this.delay.poll_unpin(cx) {}

		if let Some((ref to, ref packet)) = this.last {
			return Poll::Ready(Some((to.clone(), GossipMessage::<B>::from(packet.clone()))))
		}

		Poll::Pending
	}
}
