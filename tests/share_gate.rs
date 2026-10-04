//! `SegwitShareValidationTests` and `ShareAttributionTests` from boot-protocol `be0f0b16`.

use blvm_gridpool::{
    address_to_script, coinbase_txid_hex, decode_hex, double_sha256, encode_hex,
    expected_winners_from_coinbase, validate_share, ShareSubmission,
};

const MINER: &str = "bc1qrwsx8fs0l6z7ugp5cvzy6lhss7jlyru3kg9s8y";
const PREV: &str = "000000000000000000017aedb62d18a964ee5bc8b94fb87efca6df9d6f99431a";
const HEADER: &str = "10609d221a43996f9ddfa6fc7eb84fb9c85bee64a9182db6ed7a0100000000000000000080fed2efc47069fb944a0b2afab1d6ee5a0cf4ab60ba497769b6007d7e1f6ae692eb686ad43a0217b084bb42";
const COINBASE: &str = "020000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff380305a60e1e2f47726964506f6f6c2053746172744f53204e6174697665205356322f2f140100000000000000000000000000000000000000ffffffff0544921d00000000001600141ba063a60ffe85ee2034c3044d7ef087a5f20f9104ca1f00000000001600141ba063a60ffe85ee2034c3044d7ef087a5f20f91eaa943070000000016001409602a9c642ec02e3612e77483bc7f12fbdf4f7968052d0b00000000225120c4f639fd27fc38c962b6978567b033dcf29fe8a6560394f606dbb31388e8dfec0000000000000000266a24aa21a9ed4cb0c10afc84c1916199ed3f494388b387687f98162b1ef86a5185e4b6d3923c0120000000000000000000000000000000000000000000000000000000000000000000000000";

fn merkle() -> Vec<String> {
    [
        "3d44246b8dae5aa7f58885e0fa5f8d9c380e3b966709a37534358df60d92b5f6",
        "f7c6287eacd0aaa7e5a31b75049e42ea9910c052379b1764190bac57d4e05506",
        "e704c044a4d66ad039965196ddf22f3b00b684a9d24e389fc8c3c89a6904cd5b",
        "7381ec6ab87735b1a86863391bd4be7bda97a6001f4b374558efbb18e0b961c5",
        "61b4fda886ef8c92d96b537577594f72fb14959ba4b96e9fda104b88e8be40cf",
        "bbdb696951dab31990638a9e48f857b6fe7173af9f1f3cf6de24b10770ae031c",
        "dcef65458236793dba92336da9ce9733673700808ab72a7513ae0cfa6ca815f6",
        "170704c0a93457f41f18a88a815ebbb68ecba53b4641bb264a7b33410047fb2f",
        "9906e26a997cdbec64d8d8f78bad95b1ff9a1a2fafeaa92ddee11cd8bd8ad940",
        "92e92ba799098ebefcd622a4efd27fa7d2f84761c7f00b5820909a0c8bd25d20",
        "0ac5a345ab38334802a095abc1ce30cd9d52fd997520634100298df085275804",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[test]
fn segwit_coinbase_uses_the_transaction_id_for_the_merkle_root() {
    let txid = coinbase_txid_hex(COINBASE).expect("txid");
    assert_eq!(
        txid,
        "24a257e0ec6145333d963f90e5d34d24748a009cce6d315ea0fe229828c01f6b"
    );
    let winners = expected_winners_from_coinbase(COINBASE, "mainnet");
    let result = validate_share(
        &ShareSubmission {
            header_hex: HEADER.into(),
            coinbase_hex: COINBASE.into(),
            merkle_path: merkle(),
            prev_block_hash: PREV.into(),
            username: format!("{MINER}.sv2test"),
            expected_share_id: None,
        },
        &winners,
        &[PREV.into()],
        "mainnet",
    );
    assert!(result.valid, "{:?}", result.reason);
    assert_eq!(result.miner_address, MINER);
}

#[test]
fn a_wrong_parent_is_rejected_before_reconcile() {
    let winners = expected_winners_from_coinbase(COINBASE, "mainnet");
    let result = validate_share(
        &ShareSubmission {
            header_hex: HEADER.into(),
            coinbase_hex: COINBASE.into(),
            merkle_path: merkle(),
            prev_block_hash: PREV.into(),
            username: MINER.into(),
            expected_share_id: None,
        },
        &winners,
        &["00000000000000000000000000000000000000000000000000000000000000bb".into()],
        "mainnet",
    );
    assert!(!result.valid);
    let reason = result.reason.unwrap_or_default();
    assert!(reason.contains("wrong parent"));
}

const SLOT0: &str = "bc1qce93hy5rhg02s6aeu7mfdvxg76x66pqqtrvzs3";
const ALT: &str = "bc1qrwsx8fs0l6z7ugp5cvzy6lhss7jlyru3kg9s8y";
const SAMPLE_PREV: &str = "00000000000000000002029d47c98d2ad5c020ce9a92af8ace14b882abfa1643";
const SAMPLE_HEADER: &str = "00804f274316faab82b814ce8aaf929ace20c0d52a8dc9479d02020000000000000000002e0c639c7934a697d14a314cea5da30f0c45660248d534db3cfb2036b5ac0d8a65a6e3696913021778491e84";
const SAMPLE_COINBASE: &str = "01000000010000000000000000000000000000000000000000000000000000000000000000ffffffff2003e16d0e13426f6f742070726f746f636f6c0f626f6f74000709921015000000ffffffff06128e120000000000160014c64b1b9283ba1ea86bb9e7b696b0c8f68dad040004cc041000000000160014c64b1b9283ba1ea86bb9e7b696b0c8f68dad04000000000000000000106a0e9113b1ccf00d0000000000b9bb1952ad8b02000000001600141ba063a60ffe85ee2034c3044d7ef087a5f20f910000000000000000036a01000000000000000000266a24aa21a9edcddc611f6111ea75c5a265fba065e8eccb3d1ec8f954c738ea4586b3fffab1ce00000000";

fn sample_merkle() -> Vec<String> {
    [
        "b6c40a03e40f9f35ff1a47dfc044a0b82dced05867abda7a3d77476f8d76ca8c",
        "e8474aac0f34d17bec62afaa624ebe64b36f9ce951daca4205dfcbb3061ce1a2",
        "e93d732b034381c7746dfb0a83ff7396336f8dd454fe5a7c7999f80e8c15b2d7",
        "0276e88a8933b31dc3f8b517bae033e18416754cca5f838320b6d3ab89be7b69",
        "254f86497567f0acd39d3a31948258459505bb0634fec20a27c6d7af3a3781c8",
        "fc970819c5af1e177e882851fe7c07b8206213422322d8571c51a9ef87a8d2e5",
        "1461f6060b70b6079c7b30ca17b0dde6657704da86b88e61844be9936be5157b",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn sample_submission(header: &str, coinbase: &str) -> ShareSubmission {
    ShareSubmission {
        header_hex: header.into(),
        coinbase_hex: coinbase.into(),
        merkle_path: sample_merkle(),
        prev_block_hash: SAMPLE_PREV.into(),
        username: String::new(),
        expected_share_id: None,
    }
}

struct SerializedOutput {
    value: u64,
    bytes: Vec<u8>,
}

fn read_varint(bytes: &[u8], offset: &mut usize) -> u64 {
    let first = bytes[*offset];
    *offset += 1;
    match first {
        0xfd => {
            let value = u16::from_le_bytes(bytes[*offset..*offset + 2].try_into().unwrap());
            *offset += 2;
            u64::from(value)
        }
        0xfe => {
            let value = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
            *offset += 4;
            u64::from(value)
        }
        0xff => {
            let value = u64::from_le_bytes(bytes[*offset..*offset + 8].try_into().unwrap());
            *offset += 8;
            value
        }
        other => u64::from(other),
    }
}

fn encode_varint(value: u64) -> Vec<u8> {
    if value < 0xfd {
        vec![value as u8]
    } else if value <= u64::from(u16::MAX) {
        let mut encoded = vec![0xfd];
        encoded.extend_from_slice(&(value as u16).to_le_bytes());
        encoded
    } else if value <= u64::from(u32::MAX) {
        let mut encoded = vec![0xfe];
        encoded.extend_from_slice(&(value as u32).to_le_bytes());
        encoded
    } else {
        let mut encoded = vec![0xff];
        encoded.extend_from_slice(&value.to_le_bytes());
        encoded
    }
}

fn read_outputs(coinbase_hex: &str) -> (Vec<u8>, Vec<SerializedOutput>, Vec<u8>) {
    let tx = decode_hex(coinbase_hex).expect("coinbase");
    let mut offset = 4;
    if tx[offset] == 0x00 && tx[offset + 1] != 0x00 {
        offset += 2;
    }
    let input_count = read_varint(&tx, &mut offset);
    for _ in 0..input_count {
        offset += 36;
        let script_len = read_varint(&tx, &mut offset) as usize;
        offset += script_len + 4;
    }
    let output_count_at = offset;
    let output_count = read_varint(&tx, &mut offset);
    let mut outputs = Vec::new();
    for _ in 0..output_count {
        let start = offset;
        let value = u64::from_le_bytes(tx[offset..offset + 8].try_into().unwrap());
        offset += 8;
        let script_len = read_varint(&tx, &mut offset) as usize;
        offset += script_len;
        outputs.push(SerializedOutput {
            value,
            bytes: tx[start..offset].to_vec(),
        });
    }
    (
        tx[..output_count_at].to_vec(),
        outputs,
        tx[offset..].to_vec(),
    )
}

fn rebuild_coinbase(prefix: &[u8], outputs: &[SerializedOutput], suffix: &[u8]) -> String {
    let mut tx = prefix.to_vec();
    tx.extend(encode_varint(outputs.len() as u64));
    for output in outputs {
        tx.extend_from_slice(&output.bytes);
    }
    tx.extend_from_slice(suffix);
    encode_hex(&tx)
}

fn replace_script(bytes: &mut [u8], script: &[u8]) {
    let mut offset = 8;
    let script_len = read_varint(bytes, &mut offset) as usize;
    assert_eq!(script_len, script.len());
    bytes[offset..offset + script.len()].copy_from_slice(script);
}

fn rewrite_slot_zero(coinbase_hex: &str, address: &str) -> String {
    let tx = decode_hex(coinbase_hex).expect("coinbase");
    let script = address_to_script(address, "mainnet").expect("script");
    let mut offset = 4;
    if tx[offset] == 0x00 && tx[offset + 1] != 0x00 {
        offset += 2;
    }
    let input_count = read_varint(&tx, &mut offset);
    for _ in 0..input_count {
        offset += 36;
        let script_len = read_varint(&tx, &mut offset) as usize;
        offset += script_len + 4;
    }
    let _ = read_varint(&tx, &mut offset);
    offset += 8;
    let script_len = read_varint(&tx, &mut offset) as usize;
    assert_eq!(script_len, script.len());
    let mut rewritten = tx;
    rewritten[offset..offset + script.len()].copy_from_slice(&script);
    encode_hex(&rewritten)
}

fn coinbase_with_winner_prefix(coinbase_hex: &str, positive_winner_count: usize) -> String {
    let (prefix, outputs, suffix) = read_outputs(coinbase_hex);
    let mut copied = 0usize;
    let mut kept = vec![SerializedOutput {
        value: outputs[0].value,
        bytes: outputs[0].bytes.clone(),
    }];
    for output in outputs.iter().skip(1) {
        if output.value > 0 {
            if copied < positive_winner_count {
                kept.push(SerializedOutput {
                    value: output.value,
                    bytes: output.bytes.clone(),
                });
                copied += 1;
            }
            continue;
        }
        kept.push(SerializedOutput {
            value: output.value,
            bytes: output.bytes.clone(),
        });
    }
    assert_eq!(copied, positive_winner_count);
    rebuild_coinbase(&prefix, &kept, &suffix)
}

fn coinbase_with_only_slot_zero(coinbase_hex: &str) -> String {
    let (prefix, outputs, suffix) = read_outputs(coinbase_hex);
    rebuild_coinbase(&prefix, &outputs[..1], &suffix)
}

fn coinbase_with_mutated_first_winner(coinbase_hex: &str, address: &str) -> String {
    let (prefix, mut outputs, suffix) = read_outputs(coinbase_hex);
    let script = address_to_script(address, "mainnet").expect("script");
    let mut mutated = false;
    for (index, output) in outputs.iter_mut().enumerate() {
        if !mutated && index > 0 && output.value > 0 {
            replace_script(&mut output.bytes, &script);
            mutated = true;
        }
    }
    assert!(mutated);
    rebuild_coinbase(&prefix, &outputs, &suffix)
}

fn rewrite_header_merkle(header_hex: &str, coinbase_hex: &str, merkle: &[String]) -> String {
    let mut header = decode_hex(header_hex).expect("header");
    let coinbase = decode_hex(coinbase_hex).expect("coinbase");
    let mut current = double_sha256(&coinbase);
    for branch in merkle {
        let mut joined = current.to_vec();
        joined.extend(decode_hex(branch).expect("branch"));
        current = double_sha256(&joined);
    }
    header[36..68].copy_from_slice(&current);
    encode_hex(&header)
}

#[test]
fn slot_zero_wins_over_a_claimed_miner_address() {
    let winners = expected_winners_from_coinbase(SAMPLE_COINBASE, "mainnet");
    let forged = validate_share(
        &sample_submission(SAMPLE_HEADER, SAMPLE_COINBASE),
        &winners,
        &[SAMPLE_PREV.into()],
        "mainnet",
    );
    assert!(forged.valid, "{:?}", forged.reason);
    assert_eq!(forged.miner_address, SLOT0);
    let claimed = validate_share(
        &sample_submission(SAMPLE_HEADER, SAMPLE_COINBASE),
        &winners,
        &[SAMPLE_PREV.into()],
        "mainnet",
    );
    assert!(claimed.valid, "{:?}", claimed.reason);
    assert_eq!(forged.share_id, claimed.share_id);
}

#[test]
fn slot_zero_mutation_without_a_recomputed_header_is_rejected() {
    let winners = expected_winners_from_coinbase(SAMPLE_COINBASE, "mainnet");
    let mutated = rewrite_slot_zero(SAMPLE_COINBASE, ALT);
    let result = validate_share(
        &sample_submission(SAMPLE_HEADER, &mutated),
        &winners,
        &[SAMPLE_PREV.into()],
        "mainnet",
    );
    assert!(!result.valid);
    assert!(result.reason.unwrap_or_default().contains("merkle root"));
}

#[test]
fn truncated_coinbase_is_the_firmware_reject() {
    let winners = expected_winners_from_coinbase(SAMPLE_COINBASE, "mainnet");
    let truncated = coinbase_with_winner_prefix(SAMPLE_COINBASE, 1);
    let header = rewrite_header_merkle(SAMPLE_HEADER, &truncated, &sample_merkle());
    let result = validate_share(
        &sample_submission(&header, &truncated),
        &winners,
        &[SAMPLE_PREV.into()],
        "mainnet",
    );
    assert!(!result.valid);
    let reason = result.reason.unwrap_or_default();
    assert!(reason
        .contains("Coinbase appears truncated by miner firmware/DATUM coinbase-size selection"));
    assert!(reason.contains("matched 1 of 2 required GridPool payout outputs"));
}

#[test]
fn mutated_winner_script_is_a_generic_mismatch() {
    let winners = expected_winners_from_coinbase(SAMPLE_COINBASE, "mainnet");
    let mutated = coinbase_with_mutated_first_winner(SAMPLE_COINBASE, ALT);
    let header = rewrite_header_merkle(SAMPLE_HEADER, &mutated, &sample_merkle());
    let result = validate_share(
        &sample_submission(&header, &mutated),
        &winners,
        &[SAMPLE_PREV.into()],
        "mainnet",
    );
    assert!(!result.valid);
    let reason = result.reason.unwrap_or_default();
    assert!(reason.contains("Coinbase winners payouts do not match"));
    assert!(!reason.to_ascii_lowercase().contains("truncated"));
}

#[test]
fn single_recipient_coinbase_is_the_solo_fallback() {
    let winners = expected_winners_from_coinbase(SAMPLE_COINBASE, "mainnet");
    let solo = coinbase_with_only_slot_zero(SAMPLE_COINBASE);
    let header = rewrite_header_merkle(SAMPLE_HEADER, &solo, &sample_merkle());
    let result = validate_share(
        &sample_submission(&header, &solo),
        &winners,
        &[SAMPLE_PREV.into()],
        "mainnet",
    );
    assert!(!result.valid);
    let reason = result.reason.unwrap_or_default();
    assert!(reason.contains("non-Boot single-recipient template"));
    assert!(!reason.to_ascii_lowercase().contains("truncated"));
}
