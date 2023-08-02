use sc_utils::notification::{NotificationSender, NotificationStream, TracingKeyStr};

use crate::justification::TendermintJustification;

/// The sending half of the Tendermint justification channel(s).
///
/// Used to send notifications about justifications generated
/// at the end of a Tendermint round.
pub type TendermintJustificationSender<Block> = NotificationSender<TendermintJustification<Block>>;

/// The receiving half of the Tendermint justification channel.
///
/// Used to receive notifications about justifications generated
/// at the end of a Tendermint round.
/// The `TendermintJustificationStream` entity stores the `SharedJustificationSenders`
/// so it can be used to add more subscriptions.
pub type TendermintJustificationStream<Block> =
	NotificationStream<TendermintJustification<Block>, TendermintJustificationsTracingKey>;

/// Provides tracing key for GRANDPA justifications stream.
#[derive(Clone)]
pub struct TendermintJustificationsTracingKey;
impl TracingKeyStr for TendermintJustificationsTracingKey {
	const TRACING_KEY: &'static str = "mpsc_pbft_justification_notification_stream";
}
