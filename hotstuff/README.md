# Introduction
A HotStuff-based block finality tool implemented for substrate. Currently, it can replace Grandpa to achieve better responsiveness and faster confirmation speeds.

# Usage
Hotstuff consensus is divided into three crates:

1. hotstuff-primitives: Defines some common types and traits.
2. hotstuff-consensus: The core of the Hotstuff consensus.
3. pallet-hotstuff: On-chain state management pallet for Hotstuff consensus.

If you want to use the Hotstuff consensus from this repository in Substrate, all three of the above crates must be integrated.

## Runtime
First, we need to integrate `pallet-hotstuff` into the runtime.

```
...
    // Import Hotstuff Authority type from hotstuff_primitives
    use hotstuff_primitives::AuthorityId as HotstuffId;
...

   // Config pallet for runtime
   impl pallet_hotstuff::Config for Runtime {
        type AuthorityId = HotstuffId;

        type DisabledValidators = ();
        type MaxAuthorities = ConstU32<32>;
        type AllowMultipleBlocksPerSlot = ConstBool<false>;

        type WeightInfo = ();
   }

    // Install pallet-hotstuff in the runtime
    construct_runtime!(
    pub struct Runtime
    where
        Block = Block,
        NodeBlock = opaque::Block,
        UncheckedExtrinsic = UncheckedExtrinsic,
    {
        ....
        // Include hotstuff consensus pallet
        Hotstuff: pallet_hotstuff,
        ....
    });

    // Implement the `pallet-hotstuff` API for the runtime.
    impl_runtime_apis! {
        ...
        impl hotstuff_primitives::HotstuffApi<Block, HotstuffId> for Runtime {
            fn slot_duration() -> hotstuff_primitives::SlotDuration {
                hotstuff_primitives::SlotDuration::from_millis(Hotstuff::slot_duration())
            }

            fn authorities() -> Vec<HotstuffId> {
                Hotstuff::authorities().into_inner()
            }
        }
        ...
    }

    // Configure the session key
    impl_opaque_keys! {
        pub struct SessionKeys {
            ...
            pub hotstuff: Hotstuff,
            ...
        }
    }
```

## Client service
`hotstuff-consensus` must be launched within the node service for the Hotstuff consensus to function. In the typical Substrate node service startup process, `new_partial` and `new_full` are core functions.

```
pub fn new_partial(
	config: &Configuration,
) -> Result<
	sc_service::PartialComponents<
		FullClient,
		FullBackend,
		FullSelectChain,
		sc_consensus::DefaultImportQueue<Block, FullClient>,
		sc_transaction_pool::FullPool<Block, FullClient>,
		(
            // HotstuffBlockImport will send imported block to hotstuff consensus.
			hotstuff_consensus::HotstuffBlockImport<FullBackend, Block, FullClient>,
            //LinkHalf is something like `client`, `backend`, `network`, and other components needed by the Hotstuff consensus.
			hotstuff_consensus::LinkHalf<Block, FullClient, FullSelectChain>,
			Option<Telemetry>,
		),
	>,
	ServiceError,
> {
    ... 
    let (hotstuff_block_import, grandpa_link) = hotstuff_consensus::block_import(client.clone(), &client)?;
    ...

    return 	Ok(sc_service::PartialComponents {
		...
		other: (hotstuff_block_import, grandpa_link, telemetry),
	})
}
```

```
   pub fn new_full(config: Configuration) -> Result<TaskManager, ServiceError>{
      ...
      	let hotstuff_protocol_name = hotstuff_consensus::config::standard_name(
		    &client.block_hash(0).ok().flatten().expect("Genesis block exists; qed"),
		    &config.chain_spec,
	    );

	    net_config.add_notification_protocol    (hotstuff_consensus::config::hotstuff_peers_set_config(
	    	hotstuff_protocol_name.clone(),
	    ));
      ...  
        // Create hotstuff consensus voter & network.
        let (voter, hotstuff_network) = hotstuff_consensus::consensus::start_hotstuff(
	    	network,
	    	hotstuff_link,
	    	Arc::new(sync_service),
	    	hotstuff_protocol_name,
	    	keystore_container.keystore(),
	    )?;

        // Start hotstuff consensus voter
	    task_manager
	    	.spawn_essential_handle()
	    	.spawn_blocking("hotstuff block voter", None, voter); 

        // Start hotstuff consensus network
	    task_manager.spawn_essential_handle().spawn_blocking(
	    	"hotstuff network",
	    	None,
	    	hotstuff_network,
	    );
       ...
   }
```   
## Chain spec
Hotstuff pallet must be initialized in the ChainSpec, or it will result in an incorrect genesis.
```
    RuntimeGenesisConfig {
        ...
		hotstuff: pallet_hotstuff::GenesisConfig {
			authorities: initial_authorities.iter().map(|x| x.1.clone()).collect(),
		},
        ...
	}
```

# Summary
Even though the previous section provided a detailed description of how to integrate Hotstuff into Substrate, you may still have some confusion, or your integration of the Hotstuff consensus node may not be running correctly. You can refer to:

1. service config: https://github.com/Generative-Labs/Substrate-HotStuff/blob/main/node-template/node/src/service.rs 
2. chain spec: https://github.com/Generative-Labs/Substrate-HotStuff/blob/main/node-template/node/src/chain_spec.rs
3. runtime: https://github.com/Generative-Labs/Substrate-HotStuff/blob/main/node-template/runtime/src/lib.rs

If the problem still persists after this, you can open an issue in this repository.
