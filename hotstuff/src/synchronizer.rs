use std::{
	future::Future,
	pin::Pin,
	sync::{
		mpsc::Sender,
		Arc,
	},
	task::{Context, Poll},
	time::Duration,
};

use log::error;
use tokio::time::{interval, Instant, Interval};

use sc_client_api::Backend;
use sp_runtime::traits::Block as BlockT;

use crate::{client::ClientForHotstuff, store::Store};

pub struct Timer {
	delay: Interval,
}

impl Timer {
	pub fn new(duration: u64) -> Self {
		Self { delay: interval(Duration::from_millis(duration)) }
	}

	pub fn reset(&mut self) {
		self.delay.reset();
	}
}

impl Future for Timer {
	type Output = Instant;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.delay.poll_tick(cx)
	}
}

// Synchronizer synchronizes replicas to the same view.
pub struct Synchronizer<B: BlockT, BE: Backend<B>, C: ClientForHotstuff<B, BE>> {
	store: Store<B, BE, C>,

	timer: Timer,

	sender: Option<Sender<B::Hash>>,

	local_timeout_tx: Sender<()>,
}

impl<B, BE, C> Synchronizer<B, BE, C>
where
	B: BlockT,
	BE: Backend<B>,
	C: ClientForHotstuff<B, BE>,
{
	pub fn new(client: Arc<C>, timeout: u64, local_timeout_tx: Sender<()>) -> Self {
		Self {
			store: Store::new(client),
			timer: Timer::new(timeout),
			sender: None,
			local_timeout_tx,
		}
	}

	pub async fn start(&mut self) {
		loop {
			let res = tokio::select! {
				_ = &mut self.timer =>{
					if let Err(e) = self.local_timeout_tx.send(()){
						error!("synchronizer send local timeout signal failed, error{}", e);
						break;
					}
				}
			};
			println!("tokio select res {:#?}", res);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::Timer;

	#[tokio::test]
	pub async fn test_timer() {
		let timer = Timer::new(1000);

		timer.await;
	}
	// use super::*;

	// #[tokio::test]
	// pub async fn test_synchronizer_timer() {
	// 	let client = substrate_test_runtime_client::new();
	// 	let (tx, rx) = std::sync::mpsc::channel::<()>();
	// 	let mut synchronizer = Synchronizer::new(Arc::new(client), 100, tx);

	//     let handler2 = tokio::spawn(async move {
	// 		{
	// 			loop {
	// 				println!("handler2 begin ");
	// 				match rx.recv() {
	// 					Ok(_) => println!("recv ok"),
	// 					Err(e) => println!("recv err {}", e),
	// 				}
	// 			}
	// 		}
	// 	});

	//     println!("handler2 begin xxx");
	// 	let handler1 = tokio::spawn(async move {
	//         synchronizer.start().await;
	//     }).await;

	// 	tokio::join!(handler2);
	// }
}
