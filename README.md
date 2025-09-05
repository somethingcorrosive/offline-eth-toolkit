# Offline Ethereum Transaction Toolkit ( WIP )

---
This is a work in progress
---

---
A set of small, composable CLI utilities for building, signing, broadcasting, and inspecting Ethereum/Polygon transactions.

These tools are designed to support **offline transaction workflows**. You can build unsigned payloads on an air-gapped machine, sign them with a private key, and then broadcast the signed transaction on a separate online host. Each step is small and transparent.

---

## Tools Overview

- **`tx_builder`** → Build an **unsigned transaction preimage** (legacy or EIP-1559).
- **`tx_signer`** → Sign an unsigned payload with a private key → output signed RLP.
- **`tx_broadcaster`** → Broadcast a signed raw transaction via JSON-RPC.
- **`tx_inspector`** → Inspect and decode any RLP-encoded transaction (signed or unsigned).

---

## Installation

Clone and build:

```bash
git clone https://github.com/yourname/offline-eth-toolkit.git
cd offline-eth-toolkit
cargo build --release
```

The binaries will be available in `target/release/`:

```bash
ls target/release/tx_*
# tx_builder  tx_signer  tx_broadcaster  tx_inspector
```

You can also install directly into `$HOME/.cargo/bin`:

```bash
cargo install --path .
```

---

## Usage

### 1. Build an Unsigned Transaction

`tx_builder` creates an **unsigned transaction preimage**. You can choose between legacy (type-0) or EIP-1559 (type-2).

#### Legacy Example

```bash
./tx_builder \
  --to $ADDRESS \
  --value 0.01 \
  --gas-price 30 \
  --gas-limit 21000 \
  --nonce 5 \
  --chain-id 1 \
  --output unsigned_legacy.txt
```

This writes an **RLP preimage hex string** into `unsigned_legacy.txt`.

#### EIP-1559 Example

```bash
./tx_builder \
  --to $ADDRESS \
  --value 0.01 \
  --gas-limit 21000 \
  --nonce 1 \
  --chain-id 1 \
  --max-fee-gwei 50 \
  --priority-fee-gwei 2 \
  --eip1559 \
  --output unsigned_1559.txt
```

Optionally, print a QR code for offline transfer:

```bash
./tx_builder ... --qr
```

---

### 2. Sign an Unsigned Transaction

`tx_signer` takes a preimage file (or QR image) and signs it with your private key.

#### Sign from File

```bash
./tx_signer \
  --input unsigned_legacy.txt \
  --output signed_tx.txt \
  --private-key <hex_private_key>
```

#### Sign from QR ( WIP ) 

```bash
./tx_signer \
  --input-qr unsigned_qr.png \
  --output signed_tx.txt \
  --private-key <hex_private_key>
```

Optionally, output a QR of the signed raw transaction:

```bash
./tx_signer .. --qr
```

---

### 3. Broadcast a Signed Transaction

`tx_broadcaster` sends the signed raw transaction to a JSON-RPC node.

Example (with Infura/Alchemy RPC URL):

```bash
./tx_broadcaster \
  --input signed_tx.txt \
  --rpc-url https://mainnet.infura.io/v3/YOUR_API_KEY
```

If successful, you’ll see:

```
Transaction broadcasted successfully with hash: 0x1234abcd...
```

---

### 4. Inspect a Transaction

`tx_inspector` decodes any **RLP-encoded transaction hex** (signed or unsigned) for transparency.

Example:

```bash
./tx_inspector --input signed_tx.txt
```

Output might include(example):

```
Signed legacy transaction decoded:
From: 0x1234...
To:   0xdeadbeef...
Nonce: 5
Gas:   21000
Gas Price: 30 gwei
Value: 0.01 ETH
Data: 0x
Signature (v, r, s): (37, 0x..., 0x...)
Recovered sender: 0x1234...
```

---
Example Docker Command 
---

```shell
docker run --rm -it offline-eth-toolkit tx_builder --help

```
---
## Example Offline Workflow

1. On an **offline machine**:
    - Build unsigned tx:
      ```bash
      tx_builder --to 0xdeadbeef... --value 0.01 --gas-price 30 \
        --gas-limit 21000 --nonce 5 --chain-id 1 --output unsigned.txt --qr
      ```
    - Transfer the unsigned QR or file to the **signer device**.
    - **Working on fix for QR ingestion consistency **

2. On a **signer device** (can also be offline):
    - Sign the preimage:
      ```bash
      tx_signer --input unsigned.txt --output signed.txt --private-key <hexkey> --qr
      ```

3. On an **online machine**:
    - Broadcast the signed transaction:
      ```bash
      tx_broadcaster --input signed.txt --rpc-url https://your.rpc.provider
      ```

4. Optionally, inspect the signed transaction before broadcasting:
   ```bash
   tx_inspector --input signed.txt
   ```

---

## Security Notes

- **Never expose private keys** on an online machine.
- Use QR codes or USB sneaker-net to move unsigned/signed payloads between air-gapped and online hosts.
- Always inspect (`tx_inspector`) before signing/broadcasting to confirm transaction details.

---

## License

MIT
