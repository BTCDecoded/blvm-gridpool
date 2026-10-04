//! Header, transaction-id, and witness-address helpers used by `BootShareVerifier`.

use num_bigint::BigUint;
use num_traits::ToPrimitive;
use sha2::{Digest, Sha256};

pub fn normalize_hex(hex: &str) -> String {
    let trimmed = hex.trim();
    let without_prefix = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    without_prefix
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let normalized = normalize_hex(hex);
    if normalized.len() % 2 != 0 {
        return Err("odd hex length".into());
    }
    (0..normalized.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&normalized[index..index + 2], 16)
                .map_err(|_| "invalid hex".to_string())
        })
        .collect()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn reverse_hex(hex: &str) -> String {
    let normalized = normalize_hex(hex);
    if normalized.len() % 2 != 0 {
        return normalized;
    }
    let mut out = String::with_capacity(normalized.len());
    let bytes = normalized.as_bytes();
    let mut index = bytes.len();
    while index >= 2 {
        index -= 2;
        out.push(bytes[index] as char);
        out.push(bytes[index + 1] as char);
    }
    out
}

pub fn display_hash(hash_bytes: &[u8]) -> String {
    let mut reversed = hash_bytes.to_vec();
    reversed.reverse();
    encode_hex(&reversed)
}

pub fn hashes_equivalent(left: &str, right: &str) -> bool {
    let left = normalize_hex(left);
    let right = normalize_hex(right);
    if left.is_empty() || right.is_empty() {
        return left.eq_ignore_ascii_case(&right);
    }
    if left.eq_ignore_ascii_case(&right) {
        return true;
    }
    left.len() == 64 && right.len() == 64 && reverse_hex(&left).eq_ignore_ascii_case(&right)
}

pub fn double_sha256(data: &[u8]) -> [u8; 32] {
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

pub fn decode_compact_target(compact: u32) -> BigUint {
    let exponent = compact >> 24;
    let mantissa = compact & 0x007f_ffff;
    let negative = compact & 0x0080_0000 != 0;
    if mantissa == 0 || negative {
        return BigUint::from(0u8);
    }
    let mut value = BigUint::from(mantissa);
    let shift = 8 * (exponent as i32 - 3);
    if shift >= 0 {
        value <<= shift as usize;
    } else {
        value >>= (-shift) as usize;
    }
    value
}

pub fn header_difficulty(header_hash_le: &[u8]) -> f64 {
    let target_one = decode_compact_target(0x1d00_ffff);
    let hash_value = BigUint::from_bytes_le(header_hash_le);
    if hash_value == BigUint::from(0u8) {
        return f64::MAX;
    }
    // `(double)BigInteger` truncates toward zero. Round-to-nearest changes the state id.
    bigint_to_f64_trunc(&target_one) / bigint_to_f64_trunc(&hash_value)
}

fn bigint_to_f64_trunc(value: &BigUint) -> f64 {
    if value == &BigUint::from(0u8) {
        return 0.0;
    }
    let bit_len = value.bits();
    if bit_len <= 53 {
        return value.to_f64().unwrap_or(f64::MAX);
    }
    if bit_len - 1 + 1023 >= 2047 {
        return f64::INFINITY;
    }
    let top = value >> (bit_len - 53);
    let bytes = top.to_bytes_le();
    let mut buf = [0u8; 8];
    buf[..bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);
    let top = u64::from_le_bytes(buf);
    let mantissa = top & ((1_u64 << 52) - 1);
    let exponent = bit_len - 1 + 1023;
    f64::from_bits((exponent << 52) | mantissa)
}

pub struct TxOutput {
    pub value_sats: u64,
    pub script: Vec<u8>,
}

pub fn transaction_id(transaction: &[u8]) -> Result<[u8; 32], String> {
    let preimage = transaction_id_preimage(transaction)?;
    Ok(double_sha256(&preimage))
}

pub fn parse_outputs(transaction: &[u8]) -> Result<Vec<TxOutput>, String> {
    let mut offset = 0;
    skip(transaction, &mut offset, 4)?;
    let has_witness = has_witness(transaction, offset);
    if has_witness {
        skip(transaction, &mut offset, 2)?;
    }
    let input_count = read_varint(transaction, &mut offset)?;
    if input_count == 0 {
        return Err("Coinbase transaction has no inputs.".into());
    }
    for _ in 0..input_count {
        skip_input(transaction, &mut offset)?;
    }
    let output_count = read_varint(transaction, &mut offset)?;
    if output_count > 1024 {
        return Err("Coinbase transaction output count is unreasonable.".into());
    }
    let mut outputs = Vec::with_capacity(output_count as usize);
    for _ in 0..output_count {
        let value_sats = read_u64(transaction, &mut offset)?;
        let script_len = read_varint(transaction, &mut offset)?;
        let script = read_bytes(transaction, &mut offset, script_len)?;
        outputs.push(TxOutput { value_sats, script });
    }
    if has_witness {
        skip_witnesses(transaction, &mut offset, input_count)?;
    }
    skip(transaction, &mut offset, 4)?;
    if offset != transaction.len() {
        return Err("Coinbase transaction has trailing bytes.".into());
    }
    Ok(outputs)
}

fn transaction_id_preimage(transaction: &[u8]) -> Result<Vec<u8>, String> {
    if transaction.len() < 10 {
        return Err("Coinbase transaction is too short.".into());
    }
    let mut offset = 0;
    skip(transaction, &mut offset, 4)?;
    let has_witness = has_witness(transaction, offset);
    if has_witness {
        skip(transaction, &mut offset, 2)?;
    }
    let inputs_start = offset;
    let input_count = read_varint(transaction, &mut offset)?;
    if input_count == 0 {
        return Err("Coinbase transaction has no inputs.".into());
    }
    for _ in 0..input_count {
        skip_input(transaction, &mut offset)?;
    }
    let output_count = read_varint(transaction, &mut offset)?;
    if output_count > 1024 {
        return Err("Coinbase transaction output count is unreasonable.".into());
    }
    for _ in 0..output_count {
        skip(transaction, &mut offset, 8)?;
        let script_len = read_varint(transaction, &mut offset)?;
        skip(transaction, &mut offset, script_len)?;
    }
    let outputs_end = offset;
    if has_witness {
        skip_witnesses(transaction, &mut offset, input_count)?;
    }
    let locktime = offset;
    skip(transaction, &mut offset, 4)?;
    if offset != transaction.len() {
        return Err("Coinbase transaction has trailing bytes.".into());
    }
    if !has_witness {
        return Ok(transaction.to_vec());
    }
    let mut preimage = Vec::with_capacity(4 + (outputs_end - inputs_start) + 4);
    preimage.extend_from_slice(&transaction[..4]);
    preimage.extend_from_slice(&transaction[inputs_start..outputs_end]);
    preimage.extend_from_slice(&transaction[locktime..locktime + 4]);
    Ok(preimage)
}

fn has_witness(transaction: &[u8], offset: usize) -> bool {
    offset + 2 <= transaction.len()
        && transaction[offset] == 0x00
        && transaction[offset + 1] != 0x00
}

fn skip_input(transaction: &[u8], offset: &mut usize) -> Result<(), String> {
    skip(transaction, offset, 36)?;
    let script_len = read_varint(transaction, offset)?;
    skip(transaction, offset, script_len)?;
    skip(transaction, offset, 4)
}

fn skip_witnesses(transaction: &[u8], offset: &mut usize, input_count: u64) -> Result<(), String> {
    for _ in 0..input_count {
        let item_count = read_varint(transaction, offset)?;
        for _ in 0..item_count {
            let item_len = read_varint(transaction, offset)?;
            skip(transaction, offset, item_len)?;
        }
    }
    Ok(())
}

fn skip(transaction: &[u8], offset: &mut usize, len: u64) -> Result<(), String> {
    let len = usize::try_from(len).map_err(|_| "length overflow".to_string())?;
    if *offset + len > transaction.len() {
        return Err("Coinbase transaction is too short.".into());
    }
    *offset += len;
    Ok(())
}

fn read_bytes(transaction: &[u8], offset: &mut usize, len: u64) -> Result<Vec<u8>, String> {
    let len = usize::try_from(len).map_err(|_| "length overflow".to_string())?;
    if *offset + len > transaction.len() {
        return Err("Coinbase transaction is too short.".into());
    }
    let bytes = transaction[*offset..*offset + len].to_vec();
    *offset += len;
    Ok(bytes)
}

fn read_u64(transaction: &[u8], offset: &mut usize) -> Result<u64, String> {
    if *offset + 8 > transaction.len() {
        return Err("Coinbase transaction is too short.".into());
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&transaction[*offset..*offset + 8]);
    *offset += 8;
    Ok(u64::from_le_bytes(buf))
}

fn read_varint(transaction: &[u8], offset: &mut usize) -> Result<u64, String> {
    if *offset >= transaction.len() {
        return Err("Coinbase transaction is too short.".into());
    }
    let prefix = transaction[*offset];
    *offset += 1;
    match prefix {
        0xff => read_u64(transaction, offset),
        0xfe => {
            if *offset + 4 > transaction.len() {
                return Err("Coinbase transaction is too short.".into());
            }
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&transaction[*offset..*offset + 4]);
            *offset += 4;
            Ok(u32::from_le_bytes(buf) as u64)
        }
        0xfd => {
            if *offset + 2 > transaction.len() {
                return Err("Coinbase transaction is too short.".into());
            }
            let mut buf = [0u8; 2];
            buf.copy_from_slice(&transaction[*offset..*offset + 2]);
            *offset += 2;
            Ok(u16::from_le_bytes(buf) as u64)
        }
        other => Ok(other as u64),
    }
}

pub fn hrp_for_network(network: &str) -> &'static str {
    match network.trim().to_lowercase().as_str() {
        "testnet" | "testnet4" | "signet" => "tb",
        "regtest" => "bcrt",
        _ => "bc",
    }
}

pub fn script_to_address(script: &[u8], network: &str) -> Option<String> {
    let (version, program) = witness_program(script)?;
    encode_segwit(hrp_for_network(network), version, program)
}

pub fn address_to_script(address: &str, network: &str) -> Result<Vec<u8>, String> {
    let (version, program) = decode_segwit(hrp_for_network(network), address.trim())?;
    let mut script = Vec::with_capacity(2 + program.len());
    script.push(if version == 0 { 0x00 } else { 0x50 + version });
    script.push(program.len() as u8);
    script.extend_from_slice(&program);
    Ok(script)
}

fn witness_program(script: &[u8]) -> Option<(u8, &[u8])> {
    if script.len() < 4 {
        return None;
    }
    let version = match script[0] {
        0x00 => 0,
        opcode @ 0x51..=0x60 => opcode - 0x50,
        _ => return None,
    };
    let push = script[1] as usize;
    if push != script.len() - 2 {
        return None;
    }
    let program = &script[2..];
    if (version == 0 && (program.len() == 20 || program.len() == 32))
        || (version != 0 && (2..=40).contains(&program.len()))
    {
        Some((version, program))
    } else {
        None
    }
}

fn encode_segwit(hrp: &str, version: u8, program: &[u8]) -> Option<String> {
    let mut data = Vec::with_capacity(1 + program.len());
    data.push(version);
    data.extend(convert_bits(program, 8, 5, true)?);
    let spec = if version == 0 {
        Bech32Spec::Bech32
    } else {
        Bech32Spec::Bech32m
    };
    Some(bech32_encode(hrp, &data, spec))
}

fn decode_segwit(hrp: &str, address: &str) -> Result<(u8, Vec<u8>), String> {
    let (decoded_hrp, data, spec) = bech32_decode(address)?;
    if !decoded_hrp.eq_ignore_ascii_case(hrp) {
        return Err("address network mismatch".into());
    }
    if data.is_empty() {
        return Err("empty witness program".into());
    }
    let version = data[0];
    if version > 16 {
        return Err("bad witness version".into());
    }
    let program = convert_bits(&data[1..], 5, 8, false).ok_or("bad witness program")?;
    let expected = if version == 0 {
        Bech32Spec::Bech32
    } else {
        Bech32Spec::Bech32m
    };
    if spec != expected {
        return Err("wrong bech32 checksum".into());
    }
    Ok((version, program))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bech32Spec {
    Bech32,
    Bech32m,
}

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn bech32_polymod(values: &[u8]) -> u32 {
    let mut chk: u32 = 1;
    for value in values {
        let top = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ u32::from(*value);
        for (index, generator) in [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
            .into_iter()
            .enumerate()
        {
            if (top >> index) & 1 == 1 {
                chk ^= generator;
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(hrp.len() * 2 + 1);
    for byte in hrp.bytes() {
        out.push(byte >> 5);
    }
    out.push(0);
    for byte in hrp.bytes() {
        out.push(byte & 31);
    }
    out
}

fn bech32_encode(hrp: &str, data: &[u8], spec: Bech32Spec) -> String {
    let mut values = hrp_expand(hrp);
    values.extend_from_slice(data);
    values.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let constant = match spec {
        Bech32Spec::Bech32 => 1,
        Bech32Spec::Bech32m => 0x2bc8_30a3,
    };
    let polymod = bech32_polymod(&values) ^ constant;
    let mut checksum = [0u8; 6];
    for (index, byte) in checksum.iter_mut().enumerate() {
        *byte = ((polymod >> (5 * (5 - index))) & 31) as u8;
    }
    let mut out = String::new();
    out.push_str(hrp);
    out.push('1');
    for value in data.iter().chain(checksum.iter()) {
        out.push(CHARSET[*value as usize] as char);
    }
    out
}

fn bech32_decode(address: &str) -> Result<(String, Vec<u8>, Bech32Spec), String> {
    if address.len() < 8 || address.len() > 90 {
        return Err("bad address length".into());
    }
    let lower = address.to_lowercase();
    let split = lower.rfind('1').ok_or("missing separator")?;
    let hrp = &lower[..split];
    let data_part = &lower[split + 1..];
    if hrp.is_empty() || data_part.len() < 6 {
        return Err("bad address".into());
    }
    let mut data = Vec::with_capacity(data_part.len());
    for byte in data_part.bytes() {
        let index = CHARSET
            .iter()
            .position(|c| *c == byte)
            .ok_or("bad character")?;
        data.push(index as u8);
    }
    let mut values = hrp_expand(hrp);
    values.extend_from_slice(&data);
    let polymod = bech32_polymod(&values);
    let spec = if polymod == 1 {
        Bech32Spec::Bech32
    } else if polymod == 0x2bc8_30a3 {
        Bech32Spec::Bech32m
    } else {
        return Err("bad checksum".into());
    };
    data.truncate(data.len() - 6);
    Ok((hrp.to_string(), data, spec))
}

fn convert_bits(data: &[u8], from: u32, to: u32, pad: bool) -> Option<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let max = (1 << to) - 1;
    let mut out = Vec::new();
    for value in data {
        acc = (acc << from) | u32::from(*value);
        bits += from;
        while bits >= to {
            bits -= to;
            out.push(((acc >> bits) & max) as u8);
        }
    }
    if pad && bits > 0 {
        out.push(((acc << (to - bits)) & max) as u8);
    } else if !pad && (bits >= from || ((acc << (to - bits)) & max) != 0) {
        return None;
    }
    Some(out)
}
