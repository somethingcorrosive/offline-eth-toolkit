use clap::Parser;
use reqwest::Client;
use serde_json::json;
use std::{fs, time::Duration};
use hex;

/// CLI arguments for the transaction broadcaster
#[derive(Parser, Debug)]
#[command(name = "tx_broadcaster")]
#[command(about = "Broadcasts a signed Ethereum/Polygon transaction", arg_required_else_help = true)]
struct Args {
    /// Signed transaction file input (hex)
    #[arg(long)]
    input: String,

    /// RPC URL to broadcast to (e.g., Infura, Alchemy)
    #[arg(long)]
    rpc_url: String,

    /// RPC timeout in seconds
    #[arg(long, default_value_t = 30)]
    timeout: u64,
}

/// Broadcasts the signed transaction to the Ethereum network via JSON-RPC
async fn broadcast_transaction(rpc_url: &str, signed_tx: Vec<u8>, timeout_secs: u64) -> eyre::Result<String> {
    let client = Client::new();
    let params = vec![format!("0x{}", hex::encode(signed_tx))];

    let response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "eth_sendRawTransaction",
            "params": params,
            "id": 1
        }))
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    if let Some(error) = response.get("error") {
        return Err(eyre::eyre!("Error broadcasting transaction: {}", error));
    }

    let tx_hash = response
        .get("result")
        .and_then(|r| r.as_str())
        .ok_or_else(|| eyre::eyre!("Failed to get transaction hash"))?;

    Ok(tx_hash.to_string())
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Args::parse();

    let signed_tx_hex = fs::read_to_string(args.input)?;
    let signed_tx = hex::decode(signed_tx_hex.trim())?;

    let tx_hash = broadcast_transaction(&args.rpc_url, signed_tx, args.timeout).await?;
    println!("Transaction broadcasted successfully with hash: {}", tx_hash);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use serde_json::{json, Value};

    /// Read headers fully, then read exactly Content-Length bytes for the body.
    async fn read_http_request(socket: &mut TcpStream) -> (String, Vec<u8>) {
        let mut buf: Vec<u8> = Vec::with_capacity(8192);
        let mut tmp = [0u8; 2048];
        let header_end_seq = b"\r\n\r\n";
        let mut header_end_pos: Option<usize> = None;

        // Read until end of headers
        loop {
            let n = socket.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = find_subslice(&buf, header_end_seq) {
                header_end_pos = Some(pos + header_end_seq.len());
                break;
            }
        }

        let header_end_pos = header_end_pos.expect("incomplete HTTP headers");
        let headers_bytes = &buf[..header_end_pos];
        let mut body = buf[header_end_pos..].to_vec();

        // Parse Content-Length
        let headers_str = String::from_utf8_lossy(headers_bytes);
        let mut content_length: usize = 0;
        for line in headers_str.lines() {
            let low = line.to_ascii_lowercase();
            if let Some(rest) = low.strip_prefix("content-length:") {
                content_length = rest.trim().parse::<usize>().unwrap_or(0);
                break;
            }
        }

        // Read remaining body (if any)
        while body.len() < content_length {
            let need = content_length - body.len();
            let tmp_len = tmp.len();
            let chunk = need.min(tmp_len);
            let n = socket.read(&mut tmp[..chunk]).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }

        (headers_str.into_owned(), body)
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// Mock JSON-RPC server that validates request JSON.
    ///
    /// Checks:
    /// - method == "eth_sendRawTransaction"
    /// - params[0] == expected_param_hex
    ///
    /// If `respond_ok` is true, returns a fixed `result` (mock tx hash).
    /// Otherwise, returns a JSON-RPC `error`.
    async fn spawn_mock_rpc_server_with_validation(expected_param_hex: String, respond_ok: bool) -> (String, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mock_hash = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let mock_hash_for_task = mock_hash.clone();

        tokio::spawn(async move {
            if let Ok((mut socket, _peer)) = listener.accept().await {
                let (headers, body) = read_http_request(&mut socket).await;

                // Header sanity
                let h = headers.to_ascii_lowercase();
                assert!(h.contains("post "), "expected POST request");
                assert!(h.contains("content-type"), "missing content-type");
                assert!(h.contains("content-length"), "missing content-length");

                // Parse JSON body
                let v: Value = serde_json::from_slice(&body).expect("request body must be valid JSON");
                assert_eq!(v.get("jsonrpc").and_then(|x| x.as_str()), Some("2.0"), "jsonrpc must be 2.0");
                assert_eq!(v.get("method").and_then(|x| x.as_str()), Some("eth_sendRawTransaction"), "method mismatch");

                // Validate params[0]
                let p0 = v.get("params")
                    .and_then(|p| p.as_array())
                    .and_then(|a| a.get(0))
                    .and_then(|x| x.as_str())
                    .expect("params[0] missing or not string");
                assert_eq!(p0, expected_param_hex, "raw tx hex mismatch");

                // Compose response
                let (status_line, body_json) = if respond_ok {
                    ("HTTP/1.1 200 OK", json!({"jsonrpc": "2.0", "id": 1, "result": mock_hash_for_task }))
                } else {
                    ("HTTP/1.1 200 OK", json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": "validation failed (forced)"}}))
                };

                let body_str = body_json.to_string();
                let resp = format!(
                    "{status}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\n\r\n{body}",
                    status = status_line,
                    len = body_str.len(),
                    body = body_str
                );

                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        (format!("http://{}", addr), mock_hash)
    }

    #[tokio::test]
    async fn test_broadcast_transaction_file_input_success_with_validation() {
        // Prepare dummy signed tx file
        let signed_tx_hex = "f86c808504e3b2920082520894deadbeefdeadbeefdeadbeefdeadbeefdeadbeef88016345785d8a000080018080";
        let tmp_file_path = "temp_tx.txt";
        fs::write(tmp_file_path, signed_tx_hex).unwrap();

        // Expectation: "0x" + hex of decoded bytes
        let decoded = hex::decode(signed_tx_hex).unwrap();
        let expected_param_hex = format!("0x{}", hex::encode(&decoded));

        // Mock RPC server validates and returns fixed hash
        let (rpc_url, mock_hash) = spawn_mock_rpc_server_with_validation(expected_param_hex, true).await;

        // Exercise function under test
        let tx_hash = broadcast_transaction(&rpc_url, decoded, 5).await.unwrap();
        assert_eq!(tx_hash, mock_hash);

        let _ = fs::remove_file(tmp_file_path);
    }

    #[tokio::test]
    async fn test_broadcast_transaction_error_path_with_validation() {
        let signed_tx_hex = "f86c808504e3b2920082520894deadbeefdeadbeefdeadbeefdeadbeefdeadbeef88016345785d8a000080018080";
        let decoded = hex::decode(signed_tx_hex).unwrap();
        let expected_param_hex = format!("0x{}", hex::encode(&decoded));

        // Mock RPC server validates and returns an error
        let (rpc_url, _mock_hash) = spawn_mock_rpc_server_with_validation(expected_param_hex, false).await;

        let err = broadcast_transaction(&rpc_url, decoded, 5).await.unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("Error broadcasting transaction"), "unexpected error text: {}", msg);
    }
}
