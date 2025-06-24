# ARKive-CLI
A tool that allows users to interact with the Ark protocol directly from their terminal. It provides a streamlined way to manage your Ark wallet, conduct transactions, and interact with Ark Servers without the need for a graphical interface.

## Quick Start
### Installation
```bash
# Clone the repository
git clone https://github.com/pingu-73/arkive-sdk
cd arkive-sdk

# Build the project
cargo build --release

# Install the CLI
cargo install --path arkive-cli
```

### Default data directories
- Mac OS: `$HOME/Library/Application Support/arkive`
- Linux: `$HOME/.local/share/arkive` or `~/.Ark-cli` [unsure]
- Windows: `%APPDATA%\arkive`

### Basic Usage
```bash
# Create a new wallet
arkive wallet create my-wallet --network mutinynet

# Check balance
arkive balance show my-wallet

# Get addresses for receiving funds
arkive balance address my-wallet

# Send Ark transaction
arkive transaction send-ark my-wallet <ark-address> <amount>

# Participate in Ark round (settle transactions)
arkive ark round my-wallet

# Create encrypted backup
arkive backup create my-wallet --output backup.json
```

## CLI Basic Reference
### Wallet Management
```bash
# Create wallet
arkive wallet create <name> --network <regtest|signet|mutinynet>

# List all wallets
arkive wallet list

# Show wallet information
arkive wallet info <name>

# Delete wallet
arkive wallet delete <name>
```

### Balance Operations
```bash
# Show balance summary
arkive balance show <wallet>

# Detailed balance breakdown
arkive balance detail <wallet>

# Get receiving addresses
arkive balance address <wallet> [--address-type <onchain|ark|boarding>]
```

### Transactions
```bash
# Send on-chain Bitcoin
arkive transaction send-onchain <wallet> <address> <amount>

# Send Ark transaction (off-chain)
arkive transaction send-ark <wallet> <ark-address> <amount>

# View transaction history
arkive transaction history <wallet> [--limit 20]

# Estimate transaction fees
arkive transaction estimate-fee <wallet> <onchain|ark> <address> <amount>
```

### Ark Protocol Operations
```bash
# List VTXOs
arkive ark vtxos <wallet>

# Participate in round
arkive ark round <wallet>

# Sync with Ark server
arkive ark sync <wallet>
```

### Help
```bash
arkive help

# Command specific help
arkive <command> help
```