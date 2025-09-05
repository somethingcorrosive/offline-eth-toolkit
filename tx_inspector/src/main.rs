use clap::Parser;
use ethers::types::{transaction::eip2718::TypedTransaction, Signature};
use ethers::utils::rlp;
use std::fs;
use hex;

/// CLI to inspect an Ethereum/Polygon transaction from RLP hex
#[derive(Parser, Debug)]
#[command(name = "tx_inspector")]
#[command(about = "Inspects an RLP-encoded Ethereum/Polygon transaction", long_about = None)]
struct Args {
    /// Path to file with RLP-encoded hex transaction (signed or unsigned)
    #[arg(long)]
    input: String,
}

fn main() -> eyre::Result<()> {
    let args = Args::parse();

    println!(">> Reading transaction from: {}", args.input);
    let raw = fs::read_to_string(&args.input)?.trim().to_string();

    let hex_data = raw.chars().filter(|c| !c.is_whitespace()).collect::<String>();

    if hex_data.len() % 2 != 0 {
        return Err(eyre::eyre!("Hex string has an odd number of characters: {}", hex_data.len()));
    }

    let rlp_bytes = hex::decode(&hex_data).map_err(|e| eyre::eyre!("Failed to decode hex: {}", e))?;

    println!(">> Attempting full transaction inspection...");
    if let Ok(tx) = rlp::decode::<TypedTransaction>(&rlp_bytes) {
        println!("Transaction decoded as TypedTransaction:");
        println!("{:#?}", tx);
        return Ok(());
    }

    if let Ok(signed_tx) = rlp::decode::<ethers::types::Transaction>(&rlp_bytes) {
        println!("Signed legacy transaction decoded:");
        println!("From: {:?}", signed_tx.from);
        println!("To: {:?}", signed_tx.to);
        println!("Nonce: {:?}", signed_tx.nonce);
        println!("Gas: {:?}", signed_tx.gas);
        println!("Gas Price: {:?}", signed_tx.gas_price);
        println!("Value: {:?}", signed_tx.value);
        println!("Data: {:?}", signed_tx.input);

        println!("Signature (v, r, s): ({:?}, {:?}, {:?})", signed_tx.v, signed_tx.r, signed_tx.s);

        let sig = Signature {
            r: signed_tx.r,
            s: signed_tx.s,
            v: signed_tx.v.as_u64(),
        };

        match sig.recover(signed_tx.hash()) {
            Ok(recovered) => println!("Recovered sender: {:?}", recovered),
            Err(e) => println!("Failed to recover sender: {}", e),
        }

        return Ok(());
    }

    println!("Failed to decode transaction as typed or legacy.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use hex;

    #[test]
    fn test_even_length_hex_ok() {
        let hex_data = "deadbeef";
        assert_eq!(hex_data.len() % 2, 0);
        let decoded = hex::decode(hex_data);
        assert!(decoded.is_ok());
    }

    #[test]
    fn test_odd_length_hex_fails() {
        let hex_data = "abc";
        assert_eq!(hex_data.len() % 2, 1);
        let decoded = hex::decode(hex_data);
        assert!(decoded.is_err());
    }

    #[test]
    fn test_hex_sanitization() {
        let messy = " de ad be ef ";
        let clean: String = messy.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(clean, "deadbeef");
    }
}
