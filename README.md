# Substrate Node Template for HotStuff test

A fresh [Substrate](https://substrate.io/) node, ready for hacking :rocket:



## Getting Started

Depending on your operating system and Rust version, there might be additional packages required to compile this template.
Check the [Install](https://docs.substrate.io/install/) instructions for your platform for the most common dependencies.
Alternatively, you can use one of the [alternative installation](#alternatives-installations) options.

Clone this project and prepare to build.
```sh
git clone https://github.com/Generative-Labs/Substrate-HotStuff.git
```

### Build

Use the following command to build the node without launching it:

```sh
cd Substrate-HotStuff
cargo build --release
```

### Run Single Substrate Node

The following command starts a single-node development chain that doesn't persist state:

```sh
./target/release/node-template substrate
```

### Run Single HotStuff Node

The following command starts a single-node development chain that doesn't persist state:

```sh
./target/release/node-template hot-stuff --keys nodekey --committee committee.json --store db
```



## Template Structure

A Substrate project such as this consists of a number of components that are spread across a few directories.

### Node

A blockchain node is an application that allows users to participate in a blockchain network.
Substrate-based blockchain nodes expose a number of capabilities:

- Networking: Substrate nodes use the [`libp2p`](https://libp2p.io/) networking stack to allow the
  nodes in the network to communicate with one another.
- Consensus: Blockchains must have a way to come to [consensus](https://docs.substrate.io/fundamentals/consensus/) on the state of the network.
  Substrate makes it possible to supply custom consensus engines and also ships with several consensus mechanisms that have been built on top of [Web3 Foundation research](https://research.web3.foundation/en/latest/polkadot/NPoS/index.html).
- RPC Server: A remote procedure call (RPC) server is used to interact with Substrate nodes.

There are several files in the `node` directory.
Take special note of the following:

- [`chain_spec.rs`](./node/src/chain_spec.rs): A [chain specification](https://docs.substrate.io/build/chain-spec/) is a source code file that defines a Substrate chain's initial (genesis) state.
  Chain specifications are useful for development and testing, and critical when architecting the launch of a production chain.
  Take note of the `development_config` and `testnet_genesis` functions,.
  These functions are used to define the genesis state for the local development chain configuration.
  These functions identify some [well-known accounts](https://docs.substrate.io/reference/command-line-tools/subkey/) and use them to configure the blockchain's initial state.
- [`service.rs`](./node/src/service.rs): This file defines the node implementation.
  Take note of the libraries that this file imports and the names of the functions it invokes.
  In particular, there are references to consensus-related topics, such as the [block finalization and forks](https://docs.substrate.io/fundamentals/consensus/#finalization-and-forks) and other [consensus mechanisms](https://docs.substrate.io/fundamentals/consensus/#default-consensus-models) such as Aura for block authoring and GRANDPA for finality.



### Runtime

In Substrate, the terms "runtime" and "state transition function" are analogous.
Both terms refer to the core logic of the blockchain that is responsible for validating blocks and executing the state changes they define.
The Substrate project in this repository uses [FRAME](https://docs.substrate.io/learn/runtime-development/#frame) to construct a blockchain runtime.
FRAME allows runtime developers to declare domain-specific logic in modules called "pallets".
At the heart of FRAME is a helpful [macro language](https://docs.substrate.io/reference/frame-macros/) that makes it easy to create pallets and flexibly compose them to create blockchains that can address [a variety of needs](https://substrate.io/ecosystem/projects/).

Review the [FRAME runtime implementation](./runtime/src/lib.rs) included in this template and note the following:

- This file configures several pallets to include in the runtime.
  Each pallet configuration is defined by a code block that begins with `impl $PALLET_NAME::Config for Runtime`.
- The pallets are composed into a single runtime by way of the [`construct_runtime!`](https://paritytech.github.io/substrate/master/frame_support/macro.construct_runtime.html) macro, which is part of the [core FRAME pallet library](https://docs.substrate.io/reference/frame-pallets/#system-pallets).

### Pallets

The runtime in this project is constructed using many FRAME pallets that ship with [the Substrate repository](https://github.com/paritytech/substrate/tree/master/frame) and a template pallet that is [defined in the `pallets`](./pallets/template/src/lib.rs) directory.

A FRAME pallet is comprised of a number of blockchain primitives, including:

- Storage: FRAME defines a rich set of powerful [storage abstractions](https://docs.substrate.io/build/runtime-storage/) that makes it easy to use Substrate's efficient key-value database to manage the evolving state of a blockchain.
- Dispatchables: FRAME pallets define special types of functions that can be invoked (dispatched) from outside of the runtime in order to update its state.
- Events: Substrate uses [events](https://docs.substrate.io/build/events-and-errors/) to notify users of significant state changes.
- Errors: When a dispatchable fails, it returns an error.

Each pallet has its own `Config` trait which serves as a configuration interface to generically define the types and parameters it depends on.

## Alternatives Installations

Instead of installing dependencies and building this source directly, consider the following alternatives.

### Nix

Install [nix](https://nixos.org/) and
[nix-direnv](https://github.com/nix-community/nix-direnv) for a fully plug-and-play
experience for setting up the development environment.
To get all the correct dependencies, activate direnv `direnv allow`.

### Docker

Please follow the [Substrate Docker instructions here](https://github.com/paritytech/substrate/blob/master/docker/README.md) to build the Docker container with the Substrate Node Template binary.
