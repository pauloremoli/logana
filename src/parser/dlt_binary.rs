//! DLT binary-to-text converter.
//!
//! Parses raw DLT binary data and produces newline-delimited UTF-8 text in the same
//! format that [`super::dlt::DltParser`] expects.
//!
//! Supports three binary layouts:
//!
//! - Standard AUTOSAR DLT (storage): 16-byte storage header + standard header + payload.
//! - Wire format: concatenated DLT messages without storage headers, detected via HTYP version bits.
//! - Simplified DLT: `DLT\x01` + ECU + APID + CTID + timestamp + payload_len + payload.

use std::fmt::Write;

const DLT_MAGIC: &[u8; 4] = b"DLT\x01";
const STORAGE_HEADER_LEN: usize = 16;
const SIMPLIFIED_HEADER_LEN: usize = 22;
const STD_HEADER_MIN_LEN: usize = 4;

// Standard header bits
const UEH: u8 = 0x01; // Use Extended Header
const WEID: u8 = 0x04; // With ECU ID
const WSID: u8 = 0x08; // With Session ID
const WTMS: u8 = 0x10; // With Timestamp

// HTYP version mask: bits 5-7
const VERSION_MASK: u8 = 0xE0;
const VERSION_1: u8 = 0x20; // DLT protocol version 1

// Extended header message info
const VERBOSE_BIT: u8 = 0x01;

// Type Info bits for verbose payload decoding
const TINFO_BOOL: u32 = 0x10;
const TINFO_SINT: u32 = 0x20;
const TINFO_UINT: u32 = 0x40;
const TINFO_STRG: u32 = 0x200;
const TINFO_RAWD: u32 = 0x400;

/// Returns `true` if the buffer starts with the DLT storage header magic bytes (`DLT\x01`).
pub fn is_dlt_binary(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == DLT_MAGIC
}

/// Returns `true` if the buffer looks like concatenated DLT wire-format messages
/// (no storage header). Requires at least two consecutive messages with valid
/// version-1 HTYP fields and consistent lengths for confidence.
pub fn is_dlt_wire_format(data: &[u8]) -> bool {
    if !validate_wire_message(data) {
        return false;
    }
    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    // Require a valid second message for confidence — a single message
    // is too ambiguous (many ASCII text files start with bytes in 0x20-0x3F).
    if msg_len >= data.len() {
        return false;
    }
    validate_wire_message(&data[msg_len..])
}

fn validate_wire_message(data: &[u8]) -> bool {
    if data.len() < STD_HEADER_MIN_LEN {
        return false;
    }
    let htyp = data[0];
    if htyp & VERSION_MASK != VERSION_1 {
        return false;
    }
    // Require the extended header (UEH) — real DLT messages almost always
    // have it, and without it there's no APID/CTID to distinguish from
    // arbitrary data that happens to have version-1 bits set.
    if htyp & UEH == 0 {
        return false;
    }
    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    if msg_len < STD_HEADER_MIN_LEN || msg_len > data.len() {
        return false;
    }
    let mut min_header = 4 + 10usize; // std header + extended header
    if htyp & WEID != 0 {
        min_header += 4;
    }
    if htyp & WSID != 0 {
        min_header += 4;
    }
    if htyp & WTMS != 0 {
        min_header += 4;
    }
    if msg_len < min_header {
        return false;
    }
    // Validate that the extended header has a known message type (bits 1-3 of MSIN)
    let ext_offset = min_header - 10;
    if ext_offset < msg_len {
        let msin = data[ext_offset];
        let mtype = (msin >> 1) & 0x07;
        if mtype > 3 {
            return false;
        }
    }
    true
}

/// Returns `true` when bytes 4-15 (after the magic) are all printable ASCII or null,
/// indicating the simplified DLT layout where ECU+APID+CTID follow the magic directly.
fn is_simplified_format(data: &[u8]) -> bool {
    if data.len() < SIMPLIFIED_HEADER_LEN {
        return false;
    }
    data[4..16]
        .iter()
        .all(|&b| b == 0 || b.is_ascii_graphic() || b == b' ')
}

/// Parse all DLT messages from binary data and return newline-delimited text.
///
/// Automatically detects storage, simplified, or wire format from the first message.
pub fn convert_dlt_binary_to_text(data: &[u8]) -> Vec<u8> {
    if data.len() < STD_HEADER_MIN_LEN {
        return Vec::new();
    }

    if is_dlt_binary(data) {
        if is_simplified_format(data) {
            convert_simplified(data)
        } else {
            convert_storage(data)
        }
    } else if is_dlt_wire_format(data) {
        convert_wire(data)
    } else {
        Vec::new()
    }
}

fn convert_simplified(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut pos = 0;

    while pos + SIMPLIFIED_HEADER_LEN <= data.len() {
        if &data[pos..pos + 4] != DLT_MAGIC {
            if let Some(next) = find_next_magic(data, pos + 1) {
                pos = next;
                continue;
            }
            break;
        }

        let ecu = read_ascii4(&data[pos + 4..pos + 8]);
        let apid = read_ascii4(&data[pos + 8..pos + 12]);
        let ctid = read_ascii4(&data[pos + 12..pos + 16]);
        let ts_secs = u32::from_be_bytes([
            data[pos + 16],
            data[pos + 17],
            data[pos + 18],
            data[pos + 19],
        ]);
        let payload_len = u16::from_be_bytes([data[pos + 20], data[pos + 21]]) as usize;

        let payload_start = pos + SIMPLIFIED_HEADER_LEN;
        if payload_start + payload_len > data.len() {
            if let Some(next) = find_next_magic(data, pos + 1) {
                pos = next;
                continue;
            }
            break;
        }

        let payload_bytes = &data[payload_start..payload_start + payload_len];
        let payload_text = decode_nonverbose_payload(payload_bytes);
        let ts_str = format_storage_timestamp(ts_secs, 0);

        let mut line = String::new();
        let _ = write!(
            line,
            "{} 0 {} {} {} log info non-verbose 0 {}",
            ts_str, ecu, apid, ctid, payload_text
        );
        output.extend_from_slice(line.as_bytes());
        output.push(b'\n');

        pos = payload_start + payload_len;
    }

    output
}

fn convert_storage(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut pos = 0;

    while pos + STORAGE_HEADER_LEN <= data.len() {
        if &data[pos..pos + 4] != DLT_MAGIC {
            if let Some(next) = find_next_magic(data, pos + 1) {
                pos = next;
                continue;
            }
            break;
        }

        let secs = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
        let usecs =
            u32::from_le_bytes([data[pos + 8], data[pos + 9], data[pos + 10], data[pos + 11]]);
        let storage_ecu = read_ascii4(&data[pos + 12..pos + 16]);

        let std_start = pos + STORAGE_HEADER_LEN;
        if std_start + STD_HEADER_MIN_LEN > data.len() {
            break;
        }

        let msg_len = u16::from_be_bytes([data[std_start + 2], data[std_start + 3]]) as usize;

        if msg_len < STD_HEADER_MIN_LEN || std_start + msg_len > data.len() {
            if let Some(next) = find_next_magic(data, pos + 1) {
                pos = next;
                continue;
            }
            break;
        }

        let msg_bytes = &data[std_start..std_start + msg_len];
        let ts_str = format_storage_timestamp(secs, usecs);

        if let Some(line) = format_dlt_message(msg_bytes, &ts_str, Some(&storage_ecu)) {
            output.extend_from_slice(line.as_bytes());
            output.push(b'\n');
        }

        pos = std_start + msg_len;
    }

    output
}

fn convert_wire(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut pos = 0;

    while pos + STD_HEADER_MIN_LEN <= data.len() {
        let htyp = data[pos];
        if htyp & VERSION_MASK != VERSION_1 {
            break;
        }

        let msg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if msg_len < STD_HEADER_MIN_LEN || pos + msg_len > data.len() {
            break;
        }

        let msg_bytes = &data[pos..pos + msg_len];
        let ts_placeholder = "0000/00/00 00:00:00.000000";

        if let Some(line) = format_dlt_message(msg_bytes, ts_placeholder, None) {
            output.extend_from_slice(line.as_bytes());
            output.push(b'\n');
        }

        pos += msg_len;
    }

    output
}

/// Parse a single DLT message (standard header + optional extended header + payload)
/// and format it as a text line. `ts_str` is the pre-formatted timestamp.
/// `fallback_ecu` is used when the message doesn't contain its own ECU ID.
fn format_dlt_message(
    msg_bytes: &[u8],
    ts_str: &str,
    fallback_ecu: Option<&str>,
) -> Option<String> {
    let msg_len = msg_bytes.len();
    if msg_len < STD_HEADER_MIN_LEN {
        return None;
    }

    let htyp = msg_bytes[0];
    let mut offset = 4usize;

    let std_ecu = if htyp & WEID != 0 {
        if offset + 4 > msg_len {
            return None;
        }
        let e = read_ascii4(&msg_bytes[offset..offset + 4]);
        offset += 4;
        Some(e)
    } else {
        None
    };

    if htyp & WSID != 0 {
        if offset + 4 > msg_len {
            return None;
        }
        offset += 4;
    }

    let hw_timestamp = if htyp & WTMS != 0 {
        if offset + 4 > msg_len {
            return None;
        }
        let ts = u32::from_be_bytes([
            msg_bytes[offset],
            msg_bytes[offset + 1],
            msg_bytes[offset + 2],
            msg_bytes[offset + 3],
        ]);
        offset += 4;
        Some(ts)
    } else {
        None
    };

    let ecu_str;
    let ecu = if let Some(ref e) = std_ecu {
        e.as_str()
    } else if let Some(fb) = fallback_ecu {
        fb
    } else {
        ecu_str = "----".to_string();
        &ecu_str
    };

    let (apid, ctid, msg_type_str, subtype_str, mode_str, noar, is_verbose) = if htyp & UEH != 0 {
        if offset + 10 > msg_len {
            return None;
        }
        let msin = msg_bytes[offset];
        let num_args = msg_bytes[offset + 1];
        let app = read_ascii4(&msg_bytes[offset + 2..offset + 6]);
        let ctx = read_ascii4(&msg_bytes[offset + 6..offset + 10]);
        offset += 10;

        let verbose = msin & VERBOSE_BIT != 0;
        let mtype = (msin >> 1) & 0x07;
        let msub = (msin >> 4) & 0x0F;
        let (type_s, sub_s) = decode_type_subtype(mtype, msub);

        (
            app,
            ctx,
            type_s,
            sub_s,
            if verbose { "verbose" } else { "non-verbose" },
            num_args,
            verbose,
        )
    } else {
        (
            "----".to_string(),
            "----".to_string(),
            "----",
            "----",
            "----",
            0u8,
            false,
        )
    };

    let payload_bytes = if offset < msg_len {
        &msg_bytes[offset..msg_len]
    } else {
        &[]
    };

    let payload_text = if is_verbose {
        decode_verbose_payload(payload_bytes, noar)
    } else {
        decode_nonverbose_payload(payload_bytes)
    };

    let hw_ts = hw_timestamp.unwrap_or(0);

    let mut line = String::new();
    let _ = write!(
        line,
        "{} {} {} {} {} {} {} {} {} {}",
        ts_str, hw_ts, ecu, apid, ctid, msg_type_str, subtype_str, mode_str, noar, payload_text
    );
    Some(line)
}

fn find_next_magic(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 4 <= data.len() {
        if &data[i..i + 4] == DLT_MAGIC {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn read_ascii4(bytes: &[u8]) -> String {
    let s: String = bytes
        .iter()
        .take(4)
        .take_while(|&&b| b != 0)
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '?'
            }
        })
        .collect();
    if s.is_empty() { "----".to_string() } else { s }
}

fn format_storage_timestamp(secs: u32, usecs: u32) -> String {
    let total_secs = secs as i64;

    let mut days = total_secs / 86400;
    let mut rem = total_secs % 86400;
    if rem < 0 {
        days -= 1;
        rem += 86400;
    }

    let hours = rem / 3600;
    let minutes = (rem % 3600) / 60;
    let seconds = rem % 60;

    // Civil date from days since 1970-01-01 (Howard Hinnant algorithm)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}:{:02}.{:06}",
        year, m, d, hours, minutes, seconds, usecs
    )
}

fn decode_type_subtype(mtype: u8, msub: u8) -> (&'static str, &'static str) {
    match mtype {
        0 => {
            let sub = match msub {
                0 => "----",
                1 => "fatal",
                2 => "error",
                3 => "warn",
                4 => "info",
                5 => "debug",
                6 => "verbose",
                _ => "----",
            };
            ("log", sub)
        }
        1 => {
            let sub = match msub {
                1 => "variable",
                2 => "function_in",
                3 => "function_out",
                4 => "state",
                5 => "vfb",
                _ => "----",
            };
            ("trace", sub)
        }
        2 => {
            let sub = match msub {
                1 => "ipc",
                2 => "can",
                3 => "flexray",
                4 => "most",
                5 => "ethernet",
                6 => "someip",
                _ => "----",
            };
            ("network", sub)
        }
        3 => {
            let sub = match msub {
                1 => "request",
                2 => "response",
                _ => "----",
            };
            ("control", sub)
        }
        _ => ("----", "----"),
    }
}

fn decode_verbose_payload(data: &[u8], noar: u8) -> String {
    let mut result = Vec::new();
    let mut pos = 0;

    for _ in 0..noar {
        if pos + 4 > data.len() {
            break;
        }
        let type_info =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        let tlen = ((type_info >> 8) & 0x0F) as usize;

        if type_info & TINFO_BOOL != 0 {
            if pos < data.len() {
                result.push(if data[pos] != 0 {
                    "true".to_string()
                } else {
                    "false".to_string()
                });
                pos += 1;
            }
        } else if type_info & (TINFO_SINT | TINFO_UINT) != 0 {
            let byte_len = match tlen {
                1 => 1,
                2 => 2,
                3 => 4,
                4 => 8,
                _ => 0,
            };
            if byte_len > 0 && pos + byte_len <= data.len() {
                let val = read_int_le(&data[pos..pos + byte_len], type_info & TINFO_SINT != 0);
                result.push(val);
                pos += byte_len;
            }
        } else if type_info & TINFO_STRG != 0 {
            if pos + 2 > data.len() {
                break;
            }
            let slen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + slen > data.len() {
                break;
            }
            let str_data = &data[pos..pos + slen];
            let trimmed = if slen > 0 && str_data[slen - 1] == 0 {
                &str_data[..slen - 1]
            } else {
                str_data
            };
            match std::str::from_utf8(trimmed) {
                Ok(s) => result.push(s.to_string()),
                Err(_) => result.push(hex_dump(trimmed)),
            }
            pos += slen;
        } else if type_info & TINFO_RAWD != 0 {
            if pos + 2 > data.len() {
                break;
            }
            let rlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + rlen > data.len() {
                break;
            }
            result.push(hex_dump(&data[pos..pos + rlen]));
            pos += rlen;
        } else {
            let byte_len = match tlen {
                1 => 1,
                2 => 2,
                3 => 4,
                4 => 8,
                _ => 0,
            };
            if byte_len > 0 && pos + byte_len <= data.len() {
                result.push(hex_dump(&data[pos..pos + byte_len]));
                pos += byte_len;
            } else {
                result.push(hex_dump(&data[pos..]));
                break;
            }
        }
    }

    result.join(" ")
}

fn decode_nonverbose_payload(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(s) => {
            let trimmed = s.trim_end_matches('\0');
            trimmed.to_string()
        }
        Err(_) => hex_dump(data),
    }
}

fn read_int_le(data: &[u8], signed: bool) -> String {
    match (data.len(), signed) {
        (1, false) => format!("{}", data[0]),
        (1, true) => format!("{}", data[0] as i8),
        (2, false) => format!("{}", u16::from_le_bytes([data[0], data[1]])),
        (2, true) => format!("{}", i16::from_le_bytes([data[0], data[1]])),
        (4, false) => {
            format!(
                "{}",
                u32::from_le_bytes([data[0], data[1], data[2], data[3]])
            )
        }
        (4, true) => {
            format!(
                "{}",
                i32::from_le_bytes([data[0], data[1], data[2], data[3]])
            )
        }
        (8, false) => {
            format!(
                "{}",
                u64::from_le_bytes([
                    data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]
                ])
            )
        }
        (8, true) => {
            format!(
                "{}",
                i64::from_le_bytes([
                    data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]
                ])
            )
        }
        _ => hex_dump(data),
    }
}

fn hex_dump(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::super::dlt::DltParser;
    use super::super::types::LogFormatParser;
    use super::*;

    #[test]
    fn test_is_dlt_binary_valid() {
        let mut data = vec![b'D', b'L', b'T', 0x01];
        data.extend_from_slice(&[0u8; 12]);
        assert!(is_dlt_binary(&data));
    }

    #[test]
    fn test_is_dlt_binary_invalid() {
        assert!(!is_dlt_binary(b"NOT DLT data"));
        assert!(!is_dlt_binary(b"DLT"));
        assert!(!is_dlt_binary(b""));
    }

    #[test]
    fn test_is_dlt_binary_too_short() {
        assert!(!is_dlt_binary(b"DL"));
    }

    #[test]
    fn test_is_simplified_format() {
        let mut data = Vec::new();
        data.extend_from_slice(DLT_MAGIC);
        data.extend_from_slice(b"ECU1DEMOMAI1");
        data.extend_from_slice(&[0u8; 6]);
        assert!(is_simplified_format(&data));
    }

    #[test]
    fn test_is_not_simplified_when_binary_timestamps() {
        let mut data = Vec::new();
        data.extend_from_slice(DLT_MAGIC);
        data.extend_from_slice(&1705312245u32.to_le_bytes());
        data.extend_from_slice(&123456u32.to_le_bytes());
        data.extend_from_slice(b"ECU1");
        data.extend_from_slice(&[0u8; 6]);
        assert!(!is_simplified_format(&data));
    }

    fn build_storage_header(secs: u32, usecs: u32, ecu: &[u8; 4]) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(DLT_MAGIC);
        h.extend_from_slice(&secs.to_le_bytes());
        h.extend_from_slice(&usecs.to_le_bytes());
        h.extend_from_slice(ecu);
        h
    }

    fn build_std_header(htyp: u8, mcnt: u8, length: u16) -> Vec<u8> {
        let mut h = Vec::new();
        h.push(htyp);
        h.push(mcnt);
        h.extend_from_slice(&length.to_be_bytes());
        h
    }

    fn build_ext_header(msin: u8, noar: u8, apid: &[u8; 4], ctid: &[u8; 4]) -> Vec<u8> {
        let mut h = Vec::new();
        h.push(msin);
        h.push(noar);
        h.extend_from_slice(apid);
        h.extend_from_slice(ctid);
        h
    }

    fn build_simplified_msg(
        ecu: &[u8; 4],
        apid: &[u8; 4],
        ctid: &[u8; 4],
        ts_secs: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(DLT_MAGIC);
        data.extend_from_slice(ecu);
        data.extend_from_slice(apid);
        data.extend_from_slice(ctid);
        data.extend_from_slice(&ts_secs.to_be_bytes());
        data.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        data.extend_from_slice(payload);
        data
    }

    fn build_wire_msg(
        htyp: u8,
        mcnt: u8,
        ext: Option<(&[u8; 4], &[u8; 4], u8, u8)>,
        payload: &[u8],
    ) -> Vec<u8> {
        let ext_len = if ext.is_some() { 10 } else { 0 };
        let ecu_len = if htyp & WEID != 0 { 4 } else { 0 };
        let sid_len = if htyp & WSID != 0 { 4 } else { 0 };
        let ts_len = if htyp & WTMS != 0 { 4 } else { 0 };
        let total = 4 + ecu_len + sid_len + ts_len + ext_len + payload.len();
        let mut data = build_std_header(htyp, mcnt, total as u16);
        if htyp & WEID != 0 {
            data.extend_from_slice(b"ECU1");
        }
        if htyp & WSID != 0 {
            data.extend_from_slice(&0u32.to_be_bytes());
        }
        if htyp & WTMS != 0 {
            data.extend_from_slice(&12345u32.to_be_bytes());
        }
        if let Some((apid, ctid, msin, noar)) = ext {
            data.extend_from_slice(&build_ext_header(msin, noar, apid, ctid));
        }
        data.extend_from_slice(payload);
        data
    }

    // ——— Wire format detection ———

    #[test]
    fn test_is_dlt_wire_format_single_message_rejected() {
        let data = build_wire_msg(
            VERSION_1 | UEH,
            0,
            Some((b"APP1", b"CTX1", (4 << 4), 0)),
            b"hello",
        );
        assert!(!is_dlt_wire_format(&data));
    }

    #[test]
    fn test_is_dlt_wire_format_valid_two_messages() {
        let mut data = build_wire_msg(
            VERSION_1 | UEH,
            0,
            Some((b"APP1", b"CTX1", VERBOSE_BIT | (4 << 4), 0)),
            b"msg1",
        );
        data.extend(build_wire_msg(
            VERSION_1 | UEH,
            1,
            Some((b"APP1", b"CTX1", VERBOSE_BIT | (4 << 4), 0)),
            b"msg2",
        ));
        assert!(is_dlt_wire_format(&data));
    }

    #[test]
    fn test_is_dlt_wire_format_wrong_version() {
        let data = build_wire_msg(0x00, 0, None, b"data");
        assert!(!is_dlt_wire_format(&data));
    }

    #[test]
    fn test_is_dlt_wire_format_too_short() {
        assert!(!is_dlt_wire_format(&[0x21, 0x00]));
    }

    #[test]
    fn test_is_dlt_wire_format_not_dlt_magic() {
        // DLT\x01 starts a storage-format file, not wire format
        assert!(!is_dlt_wire_format(DLT_MAGIC));
    }

    // ——— Wire format conversion ———

    fn make_two_wire_msgs(
        htyp: u8,
        ext: Option<(&[u8; 4], &[u8; 4], u8, u8)>,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut data = build_wire_msg(htyp, 0, ext, payload);
        data.extend(build_wire_msg(htyp, 1, ext, payload));
        data
    }

    #[test]
    fn test_wire_message_with_ext_header() {
        let msin = (0 << 1) | (4 << 4); // log info non-verbose
        let data = make_two_wire_msgs(
            VERSION_1 | UEH,
            Some((b"APP1", b"CTX1", msin, 0)),
            b"wire payload",
        );
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(output.contains("APP1"), "got: {}", output);
        assert!(output.contains("CTX1"), "got: {}", output);
        assert!(output.contains("log"), "got: {}", output);
        assert!(output.contains("info"), "got: {}", output);
        assert!(output.contains("wire payload"), "got: {}", output);
    }

    #[test]
    fn test_wire_multiple_messages() {
        let msin = (0 << 1) | (4 << 4); // log info non-verbose
        let mut data = Vec::new();
        for i in 0..3u8 {
            data.extend(build_wire_msg(
                VERSION_1 | UEH,
                i,
                Some((b"APP1", b"CTX1", msin, 0)),
                format!("msg{}", i).as_bytes(),
            ));
        }
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("msg0"));
        assert!(lines[1].contains("msg1"));
        assert!(lines[2].contains("msg2"));
    }

    #[test]
    fn test_wire_with_ecu_id() {
        let msin = (0 << 1) | (4 << 4);
        let data = make_two_wire_msgs(
            VERSION_1 | UEH | WEID,
            Some((b"APP1", b"CTX1", msin, 0)),
            b"payload",
        );
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("ECU1"), "got: {}", output);
    }

    #[test]
    fn test_wire_with_timestamp() {
        let msin = (0 << 1) | (4 << 4);
        let data = make_two_wire_msgs(
            VERSION_1 | UEH | WTMS,
            Some((b"APP1", b"CTX1", msin, 0)),
            b"payload",
        );
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("12345"), "got: {}", output);
    }

    #[test]
    fn test_wire_no_extended_header_not_detected() {
        // Wire format detection requires UEH, so messages without it
        // won't be detected as wire format
        let data = make_two_wire_msgs(VERSION_1, None, b"raw payload");
        assert!(!is_dlt_wire_format(&data));
    }

    #[test]
    fn test_wire_parseable_by_dlt_text_parser() {
        let msin = (0 << 1) | (4 << 4); // log info non-verbose
        let data = make_two_wire_msgs(
            VERSION_1 | UEH | WEID,
            Some((b"APP1", b"CTX1", msin, 0)),
            b"Hello Wire",
        );
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let line = output.lines().next().unwrap();

        let parts = DltParser.parse_line(line.as_bytes()).unwrap();
        assert_eq!(parts.target, Some("APP1"));
        assert!(parts.message.unwrap().contains("Hello Wire"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "ecu" && v == "ECU1")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "ctid" && v == "CTX1")
        );
    }

    #[test]
    fn test_wire_truncated_stops_cleanly() {
        let data = build_std_header(VERSION_1, 0, 100);
        // Single truncated message — detection fails, returns empty
        let text = convert_dlt_binary_to_text(&data);
        assert!(text.is_empty());
    }

    #[test]
    fn test_wire_verbose_string_payload() {
        let test_str = b"Hello Verbose\0";
        let mut payload = Vec::new();
        let type_info: u32 = TINFO_STRG;
        payload.extend_from_slice(&type_info.to_le_bytes());
        payload.extend_from_slice(&(test_str.len() as u16).to_le_bytes());
        payload.extend_from_slice(test_str);

        let msin = VERBOSE_BIT | (0 << 1) | (4 << 4);
        let data = make_two_wire_msgs(VERSION_1 | UEH, Some((b"APP1", b"CTX1", msin, 1)), &payload);
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("Hello Verbose"), "got: {}", output);
    }

    // ——— Simplified format tests ———

    #[test]
    fn test_simplified_single_message() {
        let data = build_simplified_msg(
            b"ECU1",
            b"DEMO",
            b"MAIN",
            1774206310,
            b"Engine temperature initialized",
        );
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let line = output.lines().next().unwrap();
        assert!(output.contains("ECU1"), "got: {}", output);
        assert!(output.contains("DEMO"), "got: {}", output);
        assert!(output.contains("MAIN"), "got: {}", output);
        assert!(
            output.contains("Engine temperature initialized"),
            "got: {}",
            output
        );

        let parts = DltParser.parse_line(line.as_bytes()).unwrap();
        assert_eq!(parts.target, Some("DEMO"));
        assert!(
            parts
                .message
                .unwrap()
                .contains("Engine temperature initialized")
        );
    }

    #[test]
    fn test_simplified_multiple_messages() {
        let mut data = Vec::new();
        data.extend(build_simplified_msg(
            b"ECU1", b"DEMO", b"MAIN", 1774206310, b"msg1",
        ));
        data.extend(build_simplified_msg(
            b"ECU1", b"DEMO", b"SENS", 1774206310, b"msg2",
        ));
        data.extend(build_simplified_msg(
            b"ECU1", b"DEMO", b"CTRL", 1774206310, b"msg3",
        ));

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("MAIN"));
        assert!(lines[1].contains("SENS"));
        assert!(lines[2].contains("CTRL"));
    }

    #[test]
    fn test_simplified_truncated_payload() {
        let mut data = Vec::new();
        data.extend_from_slice(DLT_MAGIC);
        data.extend_from_slice(b"ECU1DEMOMAIN");
        data.extend_from_slice(&1774206310u32.to_be_bytes());
        data.extend_from_slice(&100u16.to_be_bytes());
        data.extend_from_slice(b"short");

        let text = convert_dlt_binary_to_text(&data);
        assert!(text.is_empty());
    }

    #[test]
    fn test_simplified_recovery_after_corruption() {
        let mut data = build_simplified_msg(b"ECU1", b"DEMO", b"CTX1", 1774206310, b"good1");
        data.extend_from_slice(b"GARBAGE");
        data.extend(build_simplified_msg(
            b"ECU1", b"DEMO", b"CTX2", 1774206310, b"good2",
        ));

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("good1"));
        assert!(lines[1].contains("good2"));
    }

    #[test]
    fn test_simplified_parseable_by_dlt_text_parser() {
        let data = build_simplified_msg(b"ECU1", b"APP1", b"CTX1", 1705312245, b"Hello DLT");
        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let line = output.lines().next().unwrap();

        let parts = DltParser.parse_line(line.as_bytes()).unwrap();
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("APP1"));
        assert!(parts.message.unwrap().contains("Hello DLT"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "ecu" && v == "ECU1")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "ctid" && v == "CTX1")
        );
    }

    #[test]
    fn test_example_dlt_file_format() {
        let mut data = Vec::new();
        data.extend(build_simplified_msg(
            b"ENG1",
            b"DEMO",
            b"MAIN",
            0x69b55d66,
            b"Engine temperature initialized",
        ));
        data.extend(build_simplified_msg(
            b"ENG1",
            b"DEMO",
            b"SENS",
            0x69b55d66,
            b"Temperature sensor unstable",
        ));
        data.extend(build_simplified_msg(
            b"ENG1",
            b"DEMO",
            b"CTRL",
            0x69b55d66,
            b"Engine temperature exceeded threshold",
        ));

        assert_eq!(data.len(), 22 + 30 + 22 + 27 + 22 + 37);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("Engine temperature initialized"));
        assert!(lines[1].contains("Temperature sensor unstable"));
        assert!(lines[2].contains("Engine temperature exceeded threshold"));
    }

    // ——— Standard (storage) format tests ———

    #[test]
    fn test_standard_parse_minimal_message() {
        let mut data = build_storage_header(1705312245, 123456, b"ECU1");
        let std_hdr = build_std_header(0x00, 0, 4);
        data.extend_from_slice(&std_hdr);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("ECU1"));
        assert!(output.contains("----"));
    }

    #[test]
    fn test_standard_parse_with_extended_header() {
        let mut data = build_storage_header(1705312245, 123456, b"ECU1");

        let htyp = UEH;
        let payload = b"Hello DLT";
        let msg_len = 4 + 10 + payload.len();
        let mut msg = build_std_header(htyp, 0, msg_len as u16);

        let msin = VERBOSE_BIT | (0 << 1) | (4 << 4);
        msg.extend_from_slice(&build_ext_header(msin, 0, b"APP1", b"CTX1"));
        msg.extend_from_slice(payload);
        data.extend_from_slice(&msg);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("ECU1"));
        assert!(output.contains("APP1"));
        assert!(output.contains("CTX1"));
        assert!(output.contains("log"));
        assert!(output.contains("info"));
        assert!(output.contains("verbose"));
    }

    #[test]
    fn test_standard_parse_with_optional_std_header_fields() {
        let mut data = build_storage_header(1705312245, 123456, b"ECU1");

        let htyp = UEH | WEID | WSID | WTMS;
        let payload = b"test payload";
        let msg_len = 4 + 4 + 4 + 4 + 10 + payload.len();
        let mut msg = build_std_header(htyp, 0, msg_len as u16);

        msg.extend_from_slice(b"ECU2");
        msg.extend_from_slice(&42u32.to_be_bytes());
        msg.extend_from_slice(&12345u32.to_be_bytes());

        let msin = VERBOSE_BIT | (0 << 1) | (4 << 4);
        msg.extend_from_slice(&build_ext_header(msin, 0, b"APP1", b"CTX1"));
        msg.extend_from_slice(payload);
        data.extend_from_slice(&msg);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("ECU2"));
    }

    #[test]
    fn test_standard_verbose_payload_string_arg() {
        let mut data = build_storage_header(1705312245, 0, b"ECU1");

        let htyp = UEH;
        let test_str = b"Hello World\0";
        let mut payload = Vec::new();
        let type_info: u32 = TINFO_STRG;
        payload.extend_from_slice(&type_info.to_le_bytes());
        payload.extend_from_slice(&(test_str.len() as u16).to_le_bytes());
        payload.extend_from_slice(test_str);

        let msg_len = 4 + 10 + payload.len();
        let mut msg = build_std_header(htyp, 0, msg_len as u16);
        let msin = VERBOSE_BIT | (0 << 1) | (4 << 4);
        msg.extend_from_slice(&build_ext_header(msin, 1, b"APP1", b"CTX1"));
        msg.extend_from_slice(&payload);
        data.extend_from_slice(&msg);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("Hello World"), "got: {}", output);
    }

    #[test]
    fn test_standard_verbose_payload_integer_arg() {
        let mut data = build_storage_header(1705312245, 0, b"ECU1");

        let htyp = UEH;
        let mut payload = Vec::new();
        let type_info: u32 = TINFO_UINT | (3 << 8);
        payload.extend_from_slice(&type_info.to_le_bytes());
        payload.extend_from_slice(&42u32.to_le_bytes());

        let msg_len = 4 + 10 + payload.len();
        let mut msg = build_std_header(htyp, 0, msg_len as u16);
        let msin = VERBOSE_BIT | (0 << 1) | (4 << 4);
        msg.extend_from_slice(&build_ext_header(msin, 1, b"APP1", b"CTX1"));
        msg.extend_from_slice(&payload);
        data.extend_from_slice(&msg);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("42"), "got: {}", output);
    }

    #[test]
    fn test_standard_nonverbose_payload_hex_fallback() {
        let mut data = build_storage_header(1705312245, 0, b"ECU1");

        let htyp = UEH;
        let payload: &[u8] = &[0xFF, 0xFE, 0x00, 0x80];

        let msg_len = 4 + 10 + payload.len();
        let mut msg = build_std_header(htyp, 0, msg_len as u16);
        let msin = (0 << 1) | (4 << 4);
        msg.extend_from_slice(&build_ext_header(msin, 0, b"APP1", b"CTX1"));
        msg.extend_from_slice(payload);
        data.extend_from_slice(&msg);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        assert!(output.contains("fffe0080"), "got: {}", output);
    }

    #[test]
    fn test_standard_truncated_message_at_end() {
        let mut data = build_storage_header(1705312245, 0, b"ECU1");
        let msg = build_std_header(0x00, 0, 100);
        data.extend_from_slice(&msg);

        let text = convert_dlt_binary_to_text(&data);
        assert!(text.is_empty() || std::str::from_utf8(&text).is_ok());
    }

    #[test]
    fn test_standard_corrupted_data_scan_forward_recovery() {
        let mut data = build_storage_header(1705312245, 0, b"ECU1");
        let msg = build_std_header(0x00, 0, 4);
        data.extend_from_slice(&msg);

        data.extend_from_slice(b"GARBAGE_DATA_HERE");

        data.extend_from_slice(&build_storage_header(1705312246, 0, b"ECU1"));
        let msg2 = build_std_header(0x00, 0, 4);
        data.extend_from_slice(&msg2);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "Should recover both messages, got: {:?}",
            lines
        );
    }

    #[test]
    fn test_standard_multiple_concatenated_messages() {
        let mut data = Vec::new();
        for i in 0..3u32 {
            data.extend_from_slice(&build_storage_header(1705312245 + i, 0, b"ECU1"));
            let msg = build_std_header(0x00, i as u8, 4);
            data.extend_from_slice(&msg);
        }

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_standard_output_parseable_by_dlt_text_parser() {
        let mut data = build_storage_header(1705312245, 123456, b"ECU1");
        let htyp = UEH;

        let test_str = b"Hello DLT\0";
        let mut payload = Vec::new();
        let type_info: u32 = TINFO_STRG;
        payload.extend_from_slice(&type_info.to_le_bytes());
        payload.extend_from_slice(&(test_str.len() as u16).to_le_bytes());
        payload.extend_from_slice(test_str);

        let msg_len = 4 + 10 + payload.len();
        let mut msg = build_std_header(htyp, 0, msg_len as u16);
        let msin = VERBOSE_BIT | (0 << 1) | (4 << 4);
        msg.extend_from_slice(&build_ext_header(msin, 1, b"APP1", b"CTX1"));
        msg.extend_from_slice(&payload);
        data.extend_from_slice(&msg);

        let text = convert_dlt_binary_to_text(&data);
        let output = std::str::from_utf8(&text).unwrap();
        let line = output.lines().next().unwrap();

        let parts = DltParser.parse_line(line.as_bytes()).unwrap();
        assert_eq!(parts.timestamp, Some("2024/01/15 09:50:45.123456"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("APP1"));
        assert!(parts.message.unwrap().contains("Hello DLT"));
    }

    // ——— Shared utility tests ———

    #[test]
    fn test_storage_header_timestamp_conversion() {
        let ts_str = format_storage_timestamp(1705312245, 123456);
        assert_eq!(ts_str, "2024/01/15 09:50:45.123456");
    }

    #[test]
    fn test_epoch_timestamp() {
        let ts_str = format_storage_timestamp(0, 0);
        assert_eq!(ts_str, "1970/01/01 00:00:00.000000");
    }

    #[test]
    fn test_empty_input() {
        let text = convert_dlt_binary_to_text(&[]);
        assert!(text.is_empty());
    }

    #[test]
    fn test_read_ascii4_with_nulls() {
        assert_eq!(read_ascii4(b"EC\0\0"), "EC");
        assert_eq!(read_ascii4(b"\0\0\0\0"), "----");
        assert_eq!(read_ascii4(b"ABCD"), "ABCD");
    }

    #[test]
    fn test_decode_type_subtype_log_levels() {
        assert_eq!(decode_type_subtype(0, 1), ("log", "fatal"));
        assert_eq!(decode_type_subtype(0, 2), ("log", "error"));
        assert_eq!(decode_type_subtype(0, 3), ("log", "warn"));
        assert_eq!(decode_type_subtype(0, 4), ("log", "info"));
        assert_eq!(decode_type_subtype(0, 5), ("log", "debug"));
        assert_eq!(decode_type_subtype(0, 6), ("log", "verbose"));
    }

    #[test]
    fn test_decode_type_subtype_other_types() {
        assert_eq!(decode_type_subtype(1, 1).0, "trace");
        assert_eq!(decode_type_subtype(2, 1).0, "network");
        assert_eq!(decode_type_subtype(3, 1).0, "control");
    }

    #[test]
    fn test_hex_dump() {
        assert_eq!(hex_dump(&[0xFF, 0x00, 0xAB]), "ff00ab");
        assert_eq!(hex_dump(&[]), "");
    }
}
