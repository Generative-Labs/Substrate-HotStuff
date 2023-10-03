use sc_chain_spec::ChainSpec;
use sc_network::types::ProtocolName;

pub(crate) const HOTSTUFF_PROTOCOL_NAME: &str = "/hotstuff";

pub fn standard_name<Hash: AsRef<[u8]>>(
	genesis_hash: &Hash,
	chain_spec: &Box<dyn ChainSpec>,
) -> ProtocolName {
	let genesis_hash = genesis_hash.as_ref();
	let chain_prefix = match chain_spec.fork_id() {
		Some(fork_id) => format!("/{}/{}", array_bytes::bytes2hex("", genesis_hash), fork_id),
		None => format!("/{}", array_bytes::bytes2hex("", genesis_hash)),
	};
	format!("{}{}", chain_prefix, HOTSTUFF_PROTOCOL_NAME).into()
}

pub fn hotstuff_peers_set_config(
	protocol_name: ProtocolName,
) -> sc_network::config::NonDefaultSetConfig {
	sc_network::config::NonDefaultSetConfig {
		notifications_protocol: protocol_name,
		fallback_names: Default::default(),
		// Notifications reach ~256kiB in size at the time of writing on Kusama and Polkadot.
		max_notification_size: 1024 * 1024,
		handshake: None,
		set_config: sc_network::config::SetConfig {
			in_peers: 0,
			out_peers: 0,
			reserved_nodes: Vec::new(),
			non_reserved_mode: sc_network::config::NonReservedPeerMode::Deny,
		},
	}
}
