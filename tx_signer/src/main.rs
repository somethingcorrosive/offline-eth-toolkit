use clap::Parser;
use eyre::{eyre, Result};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{
    transaction::eip2718::TypedTransaction, Address, Bytes, NameOrAddress, Signature, TransactionRequest, U256, U64,
};
use ethers::utils::rlp;
use qrcode::{render::unicode, EcLevel, QrCode};

use image::{GrayImage, Luma};
use quircs::Quirc;
use rqrr::PreparedImage;

use std::{fs, path::Path, str::FromStr};

#[derive(Parser, Debug)]
#[command(name = "tx_signer")]
#[command(about = "Signs an Ethereum/Polygon transaction", arg_required_else_help = true)]
struct Args {
    /// Signed transaction file output (hex, ready for eth_sendRawTransaction)
    #[arg(long)]
    output: String,

    /// Private key (hex, no 0x)
    #[arg(long)]
    private_key: String,

    /// Input unsigned transaction (hex file)
    #[arg(long, required_unless_present = "input_qr")]
    input: Option<String>,

    /// Input unsigned transaction QR code image (PNG, JPEG, etc.)
    #[arg(long, required_unless_present = "input")]
    input_qr: Option<String>,

    /// Print signed transaction as QR code
    #[arg(long)]
    qr: bool,
}

fn save_qr_to_png(qr_data: &str, filename: &str) -> Result<()> {
    let code = QrCode::with_error_correction_level(qr_data.as_bytes(), EcLevel::Q)?;
    let width = code.width();
    let pixel_size = 10;
    let border = 4;

    let img_size = ((width + border * 2) * pixel_size) as u32;
    let mut img = GrayImage::new(img_size, img_size);

    for y in 0..width {
        for x in 0..width {
            let color = match code[(x, y)] {
                qrcode::types::Color::Dark => 0,
                qrcode::types::Color::Light => 255,
            };
            for dy in 0..pixel_size {
                for dx in 0..pixel_size {
                    let px = ((x + border) * pixel_size + dx) as u32;
                    let py = ((y + border) * pixel_size + dy) as u32;
                    img.put_pixel(px, py, Luma([color]));
                }
            }
        }
    }

    img.save(filename)?;
    println!("📷 QR code saved to file: {}", filename);
    Ok(())
}

/// Accept both our legacy preimage (9 items) and EIP-1559 signing payload (0x02 + 9 items).
/// Own the `data` so we don't return references to local buffers.
enum UnsignedTx {
    Legacy {
        nonce: U256,
        gas_price: U256,
        gas_limit: U256,
        to: Address,
        value: U256,
        data: Vec<u8>,
        chain_id: U256,
    },
    Eip1559 {
        chain_id: U256,
        nonce: U256,
        max_priority_fee: U256,
        max_fee: U256,
        gas_limit: U256,
        to: Address,
        value: U256,
        data: Vec<u8>,
        // accessList enforced empty
    },
}

fn parse_unsigned(bytes: &[u8]) -> Result<UnsignedTx> {
    // Detect type-2 by leading 0x02
    if let Some((&0x02, rest)) = bytes.split_first() {
        let r = rlp::Rlp::new(rest);
        if !r.is_list() || r.item_count()? != 9 {
            return Err(eyre!("EIP-1559 signing payload must be RLP list of 9 items"));
        }
        let chain_id: U256 = r.val_at(0)?;
        let nonce: U256 = r.val_at(1)?;
        let max_priority_fee: U256 = r.val_at(2)?;
        let max_fee: U256 = r.val_at(3)?;
        let gas_limit: U256 = r.val_at(4)?;
        let to: Address = r.val_at(5)?;
        let value: U256 = r.val_at(6)?;
        let data_vec: Vec<u8> = r.val_at(7)?;
        // accessList at 8; enforce empty list
        let access_list_rlp = r.at(8)?;
        if !(access_list_rlp.is_list() && access_list_rlp.item_count()? == 0) {
            return Err(eyre!("Only empty accessList is supported in unsigned payload"));
        }

        Ok(UnsignedTx::Eip1559 {
            chain_id,
            nonce,
            max_priority_fee,
            max_fee,
            gas_limit,
            to,
            value,
            data: data_vec,
        })
    } else {
        // Legacy EIP-155 preimage: 9 items
        let r = rlp::Rlp::new(bytes);
        if !r.is_list() || r.item_count()? != 9 {
            return Err(eyre!(
                "Legacy preimage must be RLP list of 9 items: [nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0]"
            ));
        }

        let nonce: U256 = r.val_at(0)?;
        let gas_price: U256 = r.val_at(1)?;
        let gas_limit: U256 = r.val_at(2)?;
        let to: Address = r.val_at(3)?;
        let value: U256 = r.val_at(4)?;
        let data_vec: Vec<u8> = r.val_at(5)?;
        let chain_id: U256 = r.val_at(6)?;
        let r0: U256 = r.val_at(7)?;
        let s0: U256 = r.val_at(8)?;
        if !(r0.is_zero() && s0.is_zero()) {
            return Err(eyre!("Expected trailing r,s = 0,0 in legacy preimage"));
        }

        Ok(UnsignedTx::Legacy {
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            data: data_vec,
            chain_id,
        })
    }
}

fn unsigned_to_typed(utx: &UnsignedTx) -> TypedTransaction {
    match utx {
        UnsignedTx::Legacy {
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            data,
            chain_id,
        } => {
            let req = TransactionRequest {
                to: Some(NameOrAddress::Address(*to)),
                value: Some(*value),
                gas_price: Some(*gas_price),
                gas: Some(*gas_limit),
                nonce: Some(*nonce),
                chain_id: Some(U64::from(chain_id.as_u64())),
                data: Some(Bytes::from(data.clone())),
                ..Default::default()
            };
            TypedTransaction::Legacy(req.into())
        }
        UnsignedTx::Eip1559 {
            chain_id,
            nonce,
            max_priority_fee,
            max_fee,
            gas_limit,
            to,
            value,
            data,
        } => {
            use ethers::types::transaction::eip2930::AccessList;
            let mut tx1559 = ethers::types::transaction::eip1559::Eip1559TransactionRequest::new();
            tx1559 = tx1559
                .chain_id(U64::from(chain_id.as_u64()))
                .nonce(*nonce)
                .max_priority_fee_per_gas(*max_priority_fee)
                .max_fee_per_gas(*max_fee)
                .gas(*gas_limit)
                .to(*to)
                .value(*value)
                .data(Bytes::from(data.clone()))
                .access_list(AccessList::default());
            TypedTransaction::Eip1559(tx1559)
        }
    }
}

fn decode_qr_from_file(path: &Path) -> Result<String> {
    // Try quircs first
    let img = image::open(path)?.to_luma8();
    let (width, height) = img.dimensions();
    let pixels = img.as_raw();
    let mut decoder = Quirc::default();
    for code_result in decoder.identify(width as usize, height as usize, pixels) {
        if let Ok(code) = code_result {
            if let Ok(decoded) = code.decode() {
                if let Ok(text) = String::from_utf8(decoded.payload.clone()) {
                    println!("Decoded via quircs");
                    return Ok(text);
                }
            }
        }
    }

    // Fallback: rqrr
    println!("Quircs failed, falling back to rqrr...");
    let mut prepared = PreparedImage::prepare(img);
    for grid in prepared.detect_grids() {
        if let Ok((_, content)) = grid.decode() {
            println!("Decoded via rqrr");
            return Ok(content);
        }
    }

    Err(eyre!("No QR code could be decoded by quircs or rqrr"))
}

fn read_unsigned_hex(args: &Args) -> Result<String> {
    if let Some(ref qr_path) = args.input_qr {
        println!("Reading unsigned transaction from QR image: {}", qr_path);
        decode_qr_from_file(Path::new(&qr_path))
    } else {
        let path = args.input.as_ref().expect("--input is required if --input-qr is not set");
        println!("Reading unsigned transaction from file: {}", path);
        Ok(fs::read_to_string(path)?)
    }
}

fn wallet_from_hex_no0x(hexkey: &str) -> Result<LocalWallet> {
    let mut s = String::with_capacity(2 + hexkey.len());
    s.push_str("0x");
    s.push_str(hexkey);
    Ok(LocalWallet::from_str(&s)?)
}

fn main() -> Result<()> {
    let args = Args::parse();

    // 1) Wallet
    let wallet = wallet_from_hex_no0x(&args.private_key)?;
    let addr = wallet.address();
    println!("Using wallet for: {addr:?}");

    // 2) Read unsigned hex (file or QR)
    let unsigned_hex = read_unsigned_hex(&args)?;
    let unsigned_hex = unsigned_hex.trim().trim_start_matches("0x");
    let unsigned_bytes = hex::decode(unsigned_hex)?;
    println!("Unsigned payload length: {} bytes", unsigned_bytes.len());

    // 3) Parse unsigned → TypedTransaction
    let utx = parse_unsigned(&unsigned_bytes)?;
    let typed: TypedTransaction = unsigned_to_typed(&utx);

    // 4) Sign
    let sig: Signature = wallet.sign_transaction_sync(&typed)?;
    let signed_raw = typed.rlp_signed(&sig);

    // 5) Output
    let signed_hex = hex::encode(&signed_raw);
    if args.qr {
        let qr = QrCode::new(signed_hex.as_bytes())?;
        let qr_string = qr.render::<unicode::Dense1x2>().build();
        println!("{qr_string}");
        save_qr_to_png(&signed_hex, "signed_qr.png")?;
    }

    fs::write(&args.output, &signed_hex)?;
    println!("Signed transaction (hex) written to: {}", args.output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::utils::parse_units;

    #[test]
    fn sign_legacy_from_preimage() {
        // Build a minimal legacy preimage: [nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0]
        let nonce: U256 = 1u64.into();
        let gas_price: U256 = parse_units("20", "gwei").unwrap().into();
        let gas_limit: U256 = 21_000u64.into();
        let to = Address::from_str("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef").unwrap();
        let value: U256 = 0u64.into();
        let data: &[u8] = &[];
        let chain_id: U256 = 1u64.into();

        let mut s = rlp::RlpStream::new_list(9);
        s.append(&nonce);
        s.append(&gas_price);
        s.append(&gas_limit);
        s.append(&to);
        s.append(&value);
        s.append(&data);
        s.append(&chain_id);
        s.append(&0u8);
        s.append(&0u8);
        let preimage = s.out().to_vec();

        let utx = parse_unsigned(&preimage).unwrap();
        let typed = unsigned_to_typed(&utx);

        let wallet = LocalWallet::from_str("0x4c0883a69102937d6231471b5ecb4765d5e97f8e4dc6e8fa6a4de3b8a3a2f55b").unwrap();
        let sig = wallet.sign_transaction_sync(&typed).unwrap();
        let raw = typed.rlp_signed(&sig);
        assert!(!raw.is_empty(), "Signed raw should not be empty");
        // v should be 37/38 for chainId=1 (EIP-155)
        assert!(sig.v == 37u64 || sig.v == 38u64);
    }

    #[test]
    fn sign_1559_from_payload() {
        // Minimal type-2 signing payload (empty data/accessList)
        let chain_id: U256 = 1u64.into();
        let nonce: U256 = 1u64.into();
        let tip: U256 = parse_units("1", "gwei").unwrap().into();
        let max: U256 = parse_units("30", "gwei").unwrap().into();
        let gas: U256 = 21000u64.into();
        let to = Address::from_str("0x000000000000000000000000000000000000dEaD").unwrap();
        let value: U256 = 0u64.into();
        let data: &[u8] = &[];

        let mut s = rlp::RlpStream::new_list(9);
        s.append(&chain_id);
        s.append(&nonce);
        s.append(&tip);
        s.append(&max);
        s.append(&gas);
        s.append(&to);
        s.append(&value);
        s.append(&data);
        let empty = rlp::RlpStream::new_list(0).out().to_vec();
        s.append_raw(&empty, 1);

        let mut bytes = vec![0x02];
        bytes.extend_from_slice(&s.out().to_vec());

        let utx = parse_unsigned(&bytes).unwrap();
        let typed = unsigned_to_typed(&utx);

        let wallet = LocalWallet::from_str("0x4c0883a69102937d6231471b5ecb4765d5e97f8e4dc6e8fa6a4de3b8a3a2f55b").unwrap();
        let sig = wallet.sign_transaction_sync(&typed).unwrap();
        let raw = typed.rlp_signed(&sig);
        assert!(!raw.is_empty(), "Signed raw should not be empty");
        assert!(hex::encode(raw).starts_with("02f8"), "Type-2 signed should start with 0x02");
    }
}
