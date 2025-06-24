# ARKive SDK
A Rust SDK for Bitcoin and Ark protocol operations, providing a clean, wallet-centric API with multi-device synchronization.

## Overview
ARKive SDK enables developers to build Bitcoin applications with seamless support for both on-chain and off-chain transactions via the Ark protocol. The SDK provides complete wallet management, transaction handling, and multi-device synchronization while maintaining self-custody and unilateral exit guarantees.

## Features
#### Multi-Network Support
- **Regtest**
- **Signet**
- **Mutinynet** (recomended)

## Quick Start
#### Installation
```bash
# Clone the repository
git clone https://github.com/pingu-73/arkive-sdk
cd arkive-sdk

# Build the project
cargo build --release

# Install the CLI (optional)
cargo install --path arkive-cli
```

## Core-Components
- `arkive-core` Foundation library with wallet management and protocol implementations
- `arkive-cli` Command-line interface for all operations

## Contributing
Please feel free to submit a Pull Request.

## License
Dual-licensed to be compatible with the Rust project.

Licensed under the Apache License, Version 2.0 http://www.apache.org/licenses/LICENSE-2.0 or the MIT license http://opensource.org/licenses/MIT, at your option. This file may not be copied, modified, or distributed except according to those terms.