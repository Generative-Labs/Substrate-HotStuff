// //! Substrate Node Template CLI library.
// #![warn(missing_docs)]

// mod chain_spec;
// #[macro_use]
// mod service;
// mod benchmarking;
// mod cli;
// mod command;
// mod rpc;

// mod config;
// mod node;

// use crate::{
// 	config::{Committee, Export as _, Secret},
// 	node::Node,
// };
// use clap::{Parser, Subcommand};
// use consensus::Committee as ConsensusCommittee;
// use env_logger::Env;
// use futures::future::join_all;
// use log::error;
// use mempool::Committee as MempoolCommittee;
// use std::fs;
// use tokio::task::JoinHandle;

// #[derive(Parser)]
// #[clap(author, version, about, long_about = None)]
// struct Cli {
// 	/// Turn debugging information on.
// 	#[clap(short, long, action = clap::ArgAction::Count)]
// 	verbose: u8,
// 	/// The command to execute.
// 	#[clap(subcommand)]
// 	command: Command,
// }

// #[derive(Subcommand)]
// enum Command {
// 	/// Generate a new keypair.
// 	Keys {
// 		/// The file where to print the new key pair.
// 		#[clap(short, long, value_parser, value_name = "FILE")]
// 		filename: String,
// 	},
// 	/// Run a HotStuff Node.
// 	HotStuff {
// 		/// The file containing the node keys.
// 		#[clap(short, long, value_parser, value_name = "FILE")]
// 		keys: String,
// 		/// The file containing committee information.
// 		#[clap(short, long, value_parser, value_name = "FILE")]
// 		committee: String,
// 		/// Optional file containing the node parameters.
// 		#[clap(short, long, value_parser, value_name = "FILE")]
// 		parameters: Option<String>,
// 		/// The path where to create the data store.
// 		#[clap(short, long, value_parser, value_name = "PATH")]
// 		store: String,
// 	},
// 	/// Run a Substrate Node.
// 	Substrate {
// 		#[clap(short, long, value_parser, value_name = "NODETYPE")]
// 		nodetype: Option<String>,
// 	},
// }

// // #[tokio::main]
// fn main() {
// 	let cli = Cli::parse();

// 	let log_level = match cli.verbose {
// 		0 => "error",
// 		1 => "warn",
// 		2 => "info",
// 		3 => "debug",
// 		_ => "trace",
// 	};
// 	let mut logger = env_logger::Builder::from_env(Env::default().default_filter_or(log_level));

// 	match cli.command {
// 		Command::Keys { filename } =>
// 			if let Err(e) = Node::print_key_file(&filename) {
// 				error!("{}", e);
// 			},
// 		Command::HotStuff { keys, committee, parameters, store } =>
// 			match Node::new(&committee, &keys, &store, parameters) {
// 				Ok(mut node) => {
// 					println!("Starting hotstuff node.");
// 					tokio::runtime::Builder::new_current_thread()
// 						.enable_all()
// 						.build()
// 						.unwrap()
// 						.block_on(async {
// 							tokio::spawn(async move {
// 								node.analyze_block().await;
// 							})
// 							.await
// 							.expect("Failed to analyze committed blocks");
// 						})
// 				},
// 				Err(e) => error!("{}", e),
// 			},
// 		Command::Substrate { nodetype } => match start_substrate_node() {
// 			Ok(_) => {
// 				// let _ = join_all(handles).await;
// 				// let _ = start_substrate_node().await;
// 			},
// 			Err(e) => error!("Failed to start substrate node: {}", e),
// 		},
// 	}
// }

// fn deploy_testbed(nodes: u16) -> Result<Vec<JoinHandle<()>>, Box<dyn std::error::Error>> {
// 	let keys: Vec<_> = (0..nodes).map(|_| Secret::new()).collect();

// 	// Print the committee file.
// 	let epoch = 1;
// 	let mempool_committee = MempoolCommittee::new(
// 		keys.iter()
// 			.enumerate()
// 			.map(|(i, key)| {
// 				let name = key.name;
// 				let stake = 1;
// 				let front = format!("0.0.0.0:{}", 25_000 + i).parse().unwrap();
// 				let mempool = format!("0.0.0.0:{}", 25_100 + i).parse().unwrap();
// 				(name, stake, front, mempool)
// 			})
// 			.collect(),
// 		epoch,
// 	);
// 	let consensus_committee = ConsensusCommittee::new(
// 		keys.iter()
// 			.enumerate()
// 			.map(|(i, key)| {
// 				let name = key.name;
// 				let stake = 1;
// 				let addresses = format!("0.0.0.0:{}", 25_200 + i).parse().unwrap();
// 				(name, stake, addresses)
// 			})
// 			.collect(),
// 		epoch,
// 	);
// 	let committee_file = "committee.json";
// 	let _ = fs::remove_file(committee_file);
// 	Committee { mempool: mempool_committee, consensus: consensus_committee }
// 		.write(committee_file)?;

// 	// Write the key files and spawn all nodes.
// 	keys.iter()
// 		.enumerate()
// 		.map(|(i, keypair)| {
// 			let key_file = format!("node_{}.json", i);
// 			let _ = fs::remove_file(&key_file);
// 			keypair.write(&key_file)?;

// 			let store_path = format!("db_{}", i);
// 			let _ = fs::remove_dir_all(&store_path);

// 			Ok(tokio::spawn(async move {
// 				match Node::new(committee_file, &key_file, &store_path, None) {
// 					Ok(mut node) => {
// 						// Sink the commit channel.
// 						while node.commit.recv().await.is_some() {}
// 					},
// 					Err(e) => error!("{}", e),
// 				}
// 			}))
// 		})
// 		.collect::<Result<_, Box<dyn std::error::Error>>>()
// }

// fn start_substrate_node() -> sc_cli::Result<()> {
// 	command::run()
// }



//! Substrate Node Template CLI library.
#![warn(missing_docs)]

mod chain_spec;
#[macro_use]
mod service;
mod benchmarking;
mod cli;
mod command;
mod rpc;

fn main() -> sc_cli::Result<()> {
	command::run()
}