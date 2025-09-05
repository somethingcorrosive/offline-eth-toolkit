use clap::Parser;
use ethers::types::{Address, U256, NameOrAddress, Bytes, U64};
use ethers::utils::parse_units;
use qrcode::{QrCode, render::unicode};
use qrcode::types::Color;  // Corrected Color import
use rlp::RlpStream;
use std::{fs, str::FromStr};

/// CLI to build an unsigned Ethereum/Polygon transaction preimage.
/// Defaults to LEGACY (EIP-155). Use --eip1559 to build a TYPE-2 signing payload.
#[derive(Parser, Debug)]
#[command(name = "tx_builder")]
#[command(about = "Builds an unsigned Ethereum/Polygon tx preimage (legacy or EIP-1559).", arg_required_else_help = true)]
struct Args {
    /// To address
    #[arg(long)]
    to: String,

    /// Value to send (in ETH) — pass as string like "0.0015"
    #[arg(long)]
    value: String,

    /// LEGACY: gas price in gwei (string). Ignored if --eip1559 is set.
    #[arg(long, conflicts_with_all=["max_fee_gwei","priority_fee_gwei"])]
    gas_price: Option<String>,

    /// EIP-1559: max fee per gas (gwei, string). Requires --eip1559 and priority fee.
    #[arg(long, requires_all=["eip1559","priority_fee_gwei"])]
    max_fee_gwei: Option<String>,

    /// EIP-1559: max priority fee per gas (gwei, string). Requires --eip1559 and max fee.
    #[arg(long, requires_all=["eip1559","max_fee_gwei"])]
    priority_fee_gwei: Option<String>,

    /// Gas limit
    #[arg(long)]
    gas_limit: u64,

    /// Nonce
    #[arg(long)]
    nonce: u64,

    /// Chain ID
    #[arg(long)]
    chain_id: u64,

    /// Optional data payload (hex, with or without 0x)
    #[arg(long, default_value = "")]
    data: String,

    /// Output file for hex-encoded preimage
    #[arg(long)]
    output: String,

    /// Print unsigned transaction as QR code
    #[arg(long)]
    qr: bool,

    /// Build an EIP-1559 (type-2) signing payload instead of legacy (type-0)
    #[arg(long)]
    eip1559: bool,
}

fn save_qr_to_png(qr_data: &str, filename: &str) -> eyre::Result<()> {
    use image::{GrayImage, Luma};
    use qrcode::{EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(qr_data.as_bytes(), EcLevel::Q)?;
    let width = code.width();
    let pixel = 12;
    let border = 6;
    let img_size = ((width + border * 2) * pixel) as u32;
    let mut img = GrayImage::new(img_size, img_size);
    for y in 0..width {
        for x in 0..width {
            let color = match code[(x, y)] { Color::Dark => 0, Color::Light => 255 };
            for dy in 0..pixel { for dx in 0..pixel {
                let px = ((x + border) * pixel + dx) as u32;
                let py = ((y + border) * pixel + dy) as u32;
                img.put_pixel(px, py, Luma([color]));
            }}
        }
    }
    img.save(filename)?;
    println!("Unsigned transaction QR code saved to: {}", filename);
    Ok(())
}

fn hex_to_bytes_strip0x(s: &str) -> eyre::Result<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    Ok(hex::decode(s)?)
}

/// Build LEGACY (type-0) EIP-155 preimage: RLP([nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0])
fn build_legacy_preimage(
    to: Address,
    value_wei: U256,
    gas_price_wei: U256,
    gas_limit: U256,
    nonce: U256,
    chain_id: U256,
    data: &[u8],
) -> Vec<u8> {
    let mut s = RlpStream::new_list(9);
    s.append(&nonce);
    s.append(&gas_price_wei);
    s.append(&gas_limit);
    s.append(&to);
    s.append(&value_wei);
    s.append(&data);
    s.append(&chain_id);
    s.append(&0u8);
    s.append(&0u8);
    s.out().to_vec()
}

/// Build EIP-1559 (type-2) signing payload bytes = 0x02 || RLP([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList])
fn build_eip1559_signing_payload(
    chain_id: U256,
    nonce: U256,
    max_priority_fee: U256,
    max_fee: U256,
    gas_limit: U256,
    to: Address,
    value: U256,
    data: &[u8],
) -> Vec<u8> {
    let mut s = RlpStream::new_list(9);
    s.append(&chain_id);
    s.append(&nonce);
    s.append(&max_priority_fee);
    s.append(&max_fee);
    s.append(&gas_limit);
    s.append(&to);
    s.append(&value);
    s.append(&data);
    let empty = RlpStream::new_list(0).out().to_vec(); // accessList: []
    s.append_raw(&empty, 1);

    let encoded = s.out().to_vec();
    let mut out = Vec::with_capacity(1 + encoded.len());
    out.push(0x02);
    out.extend_from_slice(&encoded);
    out
}

fn main() -> eyre::Result<()> {
    let args = Args::parse();

    let to = Address::from_str(&args.to)?;
    let value_wei: U256 = parse_units(&args.value, "ether")?.into();
    let gas_limit = U256::from(args.gas_limit);
    let nonce = U256::from(args.nonce);
    let chain_id = U256::from(args.chain_id);
    let data_vec = if args.data.trim().is_empty() { Vec::<u8>::new() } else { hex_to_bytes_strip0x(&args.data)? };
    let data_bytes = Bytes::from(data_vec.clone());

    // For introspection/logging
    let mut _req = ethers::types::transaction::eip1559::Eip1559TransactionRequest::new();
    _req = _req
        .to(NameOrAddress::Address(to))
        .value(value_wei)
        .gas(gas_limit)
        .nonce(nonce)
        .chain_id(U64::from(args.chain_id))
        .data(data_bytes);

    let (rlp_bytes, label) = if args.eip1559 {
        let max_fee_gwei = args.max_fee_gwei.as_ref().ok_or_else(|| eyre::eyre!("--max-fee-gwei required with --eip1559"))?;
        let priority_fee_gwei = args.priority_fee_gwei.as_ref().ok_or_else(|| eyre::eyre!("--priority-fee-gwei required with --eip1559"))?;
        let max_fee: U256 = parse_units(max_fee_gwei, "gwei")?.into();
        let max_priority: U256 = parse_units(priority_fee_gwei, "gwei")?.into();

        _req = _req
            .max_fee_per_gas(max_fee)
            .max_priority_fee_per_gas(max_priority);

        (build_eip1559_signing_payload(
            chain_id, nonce, max_priority, max_fee, gas_limit, to, value_wei, &data_vec,
        ), "EIP-1559 (type-2) signing payload")
    } else {
        let gas_price_str = args.gas_price.as_ref().ok_or_else(|| eyre::eyre!("--gas-price is required for legacy transactions"))?;
        let gas_price_wei: U256 = parse_units(gas_price_str, "gwei")?.into();

        (build_legacy_preimage(
            to, value_wei, gas_price_wei, gas_limit, nonce, chain_id, &data_vec,
        ), "LEGACY (type-0) EIP-155 preimage")
    };

    let hex_output = hex::encode(&rlp_bytes);
    fs::write(&args.output, &hex_output)?;
    println!("Unsigned {} written to: {}", label, args.output);

    if args.qr {
        println!(">> Generating QR code for unsigned transaction...");
        let qr = QrCode::new(hex_output.as_bytes()).expect("QR code generation failed");
        let string = qr.render::<unicode::Dense1x2>().build();
        println!("{}", string);
        save_qr_to_png(&hex_output, "unsigned_qr.png")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::Rlp;
    use ethers::utils::parse_units;

    fn is_rlp_list_prefix(b: u8) -> bool { b >= 0xc0 }

    #[test]
    fn legacy_preimage_fields_are_correct() {
        // Inputs
        let to = Address::from_str("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef").unwrap();
        let value: U256 = parse_units("1.5", "ether").unwrap().into();
        let gas_price: U256 = parse_units("30", "gwei").unwrap().into();
        let gas_limit = U256::from(21_000);
        let nonce = U256::from(5);
        let chain_id = U256::from(1u64);
        let data: Vec<u8> = vec![];

        // Build & decode
        let out = build_legacy_preimage(to, value, gas_price, gas_limit, nonce, chain_id, &data);
        assert!(!out.is_empty());
        assert!(is_rlp_list_prefix(out[0]));
        let r = Rlp::new(&out);

        assert!(r.is_list());
        assert_eq!(r.item_count().unwrap(), 9);

        // [nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0]
        let d_nonce: U256 = r.val_at(0).unwrap();
        let d_gas_price: U256 = r.val_at(1).unwrap();
        let d_gas_limit: U256 = r.val_at(2).unwrap();
        let d_to: Address = r.val_at(3).unwrap();
        let d_value: U256 = r.val_at(4).unwrap();
        let d_data: Vec<u8> = r.val_at(5).unwrap();
        let d_chain_id: U256 = r.val_at(6).unwrap();

        assert_eq!(d_nonce, nonce);
        assert_eq!(d_gas_price, gas_price);
        assert_eq!(d_gas_limit, gas_limit);
        assert_eq!(d_to, to);
        assert_eq!(d_value, value);
        assert_eq!(d_data, data);
        assert_eq!(d_chain_id, chain_id);

        // Last two are empty byte-strings (0x80)
        let r7 = r.at(7).unwrap();
        let r8 = r.at(8).unwrap();
        assert!(r7.is_data() && r7.data().unwrap().is_empty(), "v placeholder must be empty bytes");
        assert!(r8.is_data() && r8.data().unwrap().is_empty(), "r placeholder must be empty bytes");
    }

    #[test]
    fn eip1559_payload_fields_are_correct() {
        // Inputs
        let to = Address::from_str("0x000000000000000000000000000000000000dEaD").unwrap();
        let value: U256 = 0u64.into();
        let tip: U256 = parse_units("2", "gwei").unwrap().into();
        let max: U256 = parse_units("100", "gwei").unwrap().into();
        let gas = U256::from(21_000);
        let nonce = U256::from(1);
        let chain_id: U256 = 80002u64.into();
        let data: Vec<u8> = vec![];

        let out = build_eip1559_signing_payload(chain_id, nonce, tip, max, gas, to, value, &data);
        assert!(!out.is_empty());
        assert_eq!(out[0], 0x02);
        assert!(is_rlp_list_prefix(out[1]));

        let r = Rlp::new(&out[1..]);
        assert!(r.is_list());
        assert_eq!(r.item_count().unwrap(), 9);

        // [chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList]
        let d_chain_id: U256 = r.val_at(0).unwrap();
        let d_nonce: U256 = r.val_at(1).unwrap();
        let d_tip: U256 = r.val_at(2).unwrap();
        let d_max: U256 = r.val_at(3).unwrap();
        let d_gas: U256 = r.val_at(4).unwrap();
        let d_to: Address = r.val_at(5).unwrap();
        let d_value: U256 = r.val_at(6).unwrap();
        let d_data: Vec<u8> = r.val_at(7).unwrap();

        assert_eq!(d_chain_id, chain_id);
        assert_eq!(d_nonce, nonce);
        assert_eq!(d_tip, tip);
        assert_eq!(d_max, max);
        assert_eq!(d_gas, gas);
        assert_eq!(d_to, to);
        assert_eq!(d_value, value);
        assert_eq!(d_data, data);

        let access_list = r.at(8).unwrap();
        assert!(access_list.is_list());
        assert_eq!(access_list.item_count().unwrap(), 0);
    }
}
