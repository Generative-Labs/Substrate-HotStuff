use sc_utils::notification::{NotificationSender, NotificationStream, TracingKeyStr};

use crate::justification::GrandpaJustification;

/// The sending half of the Grandpa justification channel(s).
///
/// Used to send notifications about justifications generated
/// at the end of a Grandpa round.
pub type GrandpaJustificationSender<Block> = NotificationSender<GrandpaJustification<Block>>;

/// The receiving half of the Grandpa justification channel.
///
/// Used to receive notifications about justifications generated
/// at the end of a Grandpa round.
/// The `GrandpaJustificationStream` entity stores the `SharedJustificationSenders`
/// so it can be used to add more subscriptions.
pub type GrandpaJustificationStream<Block> =
	NotificationStream<GrandpaJustification<Block>, GrandpaJustificationsTracingKey>;

/// Provides tracing key for GRANDPA justifications stream.
#[derive(Clone)]
pub struct GrandpaJustificationsTracingKey;
impl TracingKeyStr for GrandpaJustificationsTracingKey {
	const TRACING_KEY: &'static str = "mpsc_grandpa_justification_notification_stream";
}
