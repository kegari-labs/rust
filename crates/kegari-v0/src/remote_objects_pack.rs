//! Bounded Git wire framing and pre-index expansion validation. This does not
//! grant repository authority or replace Git's strict object/link verification.
use flate2::{Decompress, FlushDecompress, Status};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectLimits {
    pub pack_bytes: usize,
    pub objects: usize,
    pub object_bytes: usize,
    pub expanded_bytes: usize,
    pub delta_depth: usize,
    pub protocol_bytes: usize,
}
impl Default for ObjectLimits {
    fn default() -> Self {
        Self {
            pack_bytes: 64 << 20,
            objects: 100_000,
            object_bytes: 16 << 20,
            expanded_bytes: 128 << 20,
            delta_depth: 64,
            protocol_bytes: 1 << 20,
        }
    }
}
impl ObjectLimits {
    pub fn validate(&self) -> Result<()> {
        if self.pack_bytes == 0
            || self.pack_bytes > 256 << 20
            || self.objects == 0
            || self.objects > 100_000
            || self.object_bytes == 0
            || self.object_bytes > 64 << 20
            || self.expanded_bytes == 0
            || self.expanded_bytes > 256 << 20
            || self.delta_depth == 0
            || self.delta_depth > 64
            || self.protocol_bytes == 0
            || self.protocol_bytes > 1 << 20
            || self.pack_bytes.saturating_add(self.protocol_bytes) > 256 << 20
        {
            return Err(Error::invalid(
                "object acquisition limits must be positive and bounded",
            ));
        }
        Ok(())
    }
}
fn invalid(message: &str) -> Error {
    Error::new("invalid_git_pack", message)
}
fn limit() -> Error {
    Error::new(
        "source_limit_exceeded",
        "Git object acquisition exceeds its finite bound",
    )
}

enum Packet<'a> {
    Data(&'a [u8]),
    Flush,
    Delimiter,
    End,
}
fn packet<'a>(input: &'a [u8], offset: &mut usize) -> Result<Packet<'a>> {
    let h = input
        .get(*offset..offset.checked_add(4).ok_or_else(limit)?)
        .ok_or_else(|| invalid("truncated packet length"))?;
    if !h.iter().all(u8::is_ascii_hexdigit) {
        return Err(invalid("invalid packet length"));
    }
    let length = usize::from_str_radix(
        std::str::from_utf8(h).map_err(|_| invalid("packet encoding"))?,
        16,
    )
    .map_err(|_| invalid("packet length overflow"))?;
    *offset += 4;
    match length {
        0 => Ok(Packet::Flush),
        1 => Ok(Packet::Delimiter),
        2 => Ok(Packet::End),
        4..=65520 => {
            let end = offset.checked_add(length - 4).ok_or_else(limit)?;
            let data = input
                .get(*offset..end)
                .ok_or_else(|| invalid("truncated packet data"))?;
            *offset = end;
            Ok(Packet::Data(data))
        }
        _ => Err(invalid("unsupported packet length")),
    }
}
pub(super) fn validate_advertisement(bytes: &[u8], limits: &ObjectLimits) -> Result<()> {
    limits.validate()?;
    if bytes.len() > limits.protocol_bytes {
        return Err(limit());
    }
    let mut offset = 0;
    let mut first = packet(bytes, &mut offset)?;
    if matches!(first, Packet::Data(b"# service=git-upload-pack\n")) {
        if !matches!(packet(bytes, &mut offset)?, Packet::Flush) {
            return Err(invalid("service advertisement framing"));
        }
        first = packet(bytes, &mut offset)?;
    }
    if !matches!(first, Packet::Data(b"version 2\n")) {
        return Err(Error::new(
            "unsupported_git_protocol",
            "Git protocol v2 is required",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut fetch = false;
    let mut terminal = false;
    while offset < bytes.len() {
        match packet(bytes, &mut offset)? {
            Packet::Data(line) if !terminal => {
                let line = std::str::from_utf8(line)
                    .map_err(|_| invalid("capability encoding"))?
                    .trim_end_matches('\n');
                let (name, value) = line.split_once('=').unwrap_or((line, ""));
                if name.is_empty() || !seen.insert(name.to_owned()) {
                    return Err(invalid("duplicate capability"));
                }
                if name == "fetch" {
                    fetch = true;
                }
                if name == "object-format" && value != "sha1" {
                    return Err(Error::new(
                        "unsupported_git_protocol",
                        "only exact SHA-1 Git objects are supported",
                    ));
                }
            }
            Packet::Flush if offset == bytes.len() => terminal = true,
            _ => return Err(invalid("unexpected capability framing")),
        }
    }
    if !fetch || !terminal {
        return Err(invalid("complete fetch capability advertisement required"));
    }
    Ok(())
}
pub(super) fn fetch_request(commits: &BTreeSet<String>) -> Result<Vec<u8>> {
    if commits.is_empty() || commits.len() > 64 {
        return Err(limit());
    }
    fn append(out: &mut Vec<u8>, line: &str) {
        out.extend_from_slice(format!("{:04x}", line.len() + 4).as_bytes());
        out.extend_from_slice(line.as_bytes());
    }
    let mut out = Vec::new();
    append(&mut out, "command=fetch\n");
    out.extend_from_slice(b"0001");
    for commit in commits {
        kegari_git::validate_oid(commit)?;
        append(&mut out, &format!("want {commit}\n"));
    }
    append(&mut out, "done\n");
    append(&mut out, "no-progress\n");
    out.extend_from_slice(b"0000");
    Ok(out)
}
pub(super) fn extract_pack(response: &[u8], limits: &ObjectLimits) -> Result<Vec<u8>> {
    limits.validate()?;
    if response.len()
        > limits
            .pack_bytes
            .checked_add(limits.protocol_bytes)
            .ok_or_else(limit)?
    {
        return Err(limit());
    }
    let mut offset = 0;
    if !matches!(packet(response, &mut offset)?, Packet::Data(b"packfile\n")) {
        return Err(invalid(
            "only a complete self-contained packfile response is accepted",
        ));
    }
    let mut pack = Vec::new();
    let mut overhead = offset;
    let mut terminal = false;
    while offset < response.len() {
        let before = offset;
        match packet(response, &mut offset)? {
            Packet::Data(data) if !terminal => match data.split_first() {
                Some((1, bytes)) => {
                    if pack.len().saturating_add(bytes.len()) > limits.pack_bytes {
                        return Err(limit());
                    }
                    pack.extend_from_slice(bytes);
                    overhead += 5;
                }
                Some((2, _)) => overhead += offset - before,
                Some((3, _)) => {
                    return Err(Error::new(
                        "source_unavailable",
                        "Git server rejected exact object acquisition",
                    ));
                }
                _ => return Err(invalid("unknown pack sideband")),
            },
            Packet::Flush | Packet::End if offset == response.len() => {
                terminal = true;
                overhead += 4;
            }
            _ => return Err(invalid("unexpected pack response section")),
        }
        if overhead > limits.protocol_bytes {
            return Err(limit());
        }
    }
    if !terminal {
        return Err(invalid("unterminated pack response"));
    }
    Ok(pack)
}

enum Base {
    Direct(u8),
    Offset(usize),
    Hash([u8; 20]),
}
struct Entry {
    offset: usize,
    base: Base,
    data: Vec<u8>,
}
struct Object {
    kind: u8,
    data: Vec<u8>,
    depth: usize,
}
pub(super) struct ValidatedPack {
    pub bytes: Vec<u8>,
    pub objects: BTreeSet<String>,
    pub expanded_bytes: usize,
}
fn size(input: &[u8], pos: &mut usize) -> Result<usize> {
    let mut value = 0usize;
    let mut shift = 0;
    loop {
        let b = *input
            .get(*pos)
            .ok_or_else(|| invalid("truncated delta size"))?;
        *pos += 1;
        if shift >= usize::BITS || (usize::from(b & 127) > (usize::MAX >> shift)) {
            return Err(limit());
        }
        value |= usize::from(b & 127) << shift;
        if b & 128 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}
fn inflate(input: &[u8], declared: usize) -> Result<(Vec<u8>, usize)> {
    let mut decoder = Decompress::new(true);
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                &input[before_in as usize..],
                &mut buffer,
                FlushDecompress::None,
            )
            .map_err(|_| invalid("invalid zlib object"))?;
        let produced = (decoder.total_out() - before_out) as usize;
        if output.len().saturating_add(produced) > declared {
            return Err(invalid("object exceeds its declared size"));
        }
        output.extend_from_slice(&buffer[..produced]);
        if status == Status::StreamEnd {
            if output.len() != declared {
                return Err(invalid("object size mismatch"));
            }
            return Ok((output, decoder.total_in() as usize));
        }
        if decoder.total_in() == before_in && produced == 0 {
            return Err(invalid("truncated zlib object"));
        }
    }
}
fn apply_delta(base: &[u8], delta: &[u8], max: usize) -> Result<Vec<u8>> {
    let mut pos = 0;
    let source = size(delta, &mut pos)?;
    let result = size(delta, &mut pos)?;
    if source != base.len() {
        return Err(invalid("delta base size mismatch"));
    }
    if result > max {
        return Err(limit());
    }
    let mut out = Vec::new();
    while pos < delta.len() {
        let command = delta[pos];
        pos += 1;
        if command & 128 != 0 {
            let mut offset = 0usize;
            let mut length = 0usize;
            for bit in 0..7 {
                if command & (1 << bit) != 0 {
                    let b = usize::from(
                        *delta
                            .get(pos)
                            .ok_or_else(|| invalid("truncated delta copy"))?,
                    );
                    pos += 1;
                    if bit < 4 {
                        offset |= b << (8 * bit);
                    } else {
                        length |= b << (8 * (bit - 4));
                    }
                }
            }
            if length == 0 {
                length = 0x10000;
            }
            let bytes = base
                .get(offset..offset.checked_add(length).ok_or_else(limit)?)
                .ok_or_else(|| invalid("delta copy escaped base"))?;
            if out.len().saturating_add(length) > result {
                return Err(invalid("delta copy exceeds result"));
            }
            out.extend_from_slice(bytes);
        } else if command != 0 {
            let end = pos.checked_add(usize::from(command)).ok_or_else(limit)?;
            let bytes = delta
                .get(pos..end)
                .ok_or_else(|| invalid("truncated delta insert"))?;
            if out.len().saturating_add(bytes.len()) > result {
                return Err(invalid("delta insert exceeds result"));
            }
            out.extend_from_slice(bytes);
            pos = end;
        } else {
            return Err(invalid("reserved delta instruction"));
        }
    }
    if out.len() != result {
        return Err(invalid("delta result size mismatch"));
    }
    Ok(out)
}
fn object_id(kind: u8, data: &[u8]) -> Result<[u8; 20]> {
    let name = match kind {
        1 => "commit",
        2 => "tree",
        3 => "blob",
        4 => "tag",
        _ => return Err(invalid("invalid object type")),
    };
    let mut hash = Sha1::new();
    hash.update(format!("{name} {}\0", data.len()));
    hash.update(data);
    Ok(hash.finalize().into())
}
pub(super) fn validate_pack(bytes: Vec<u8>, limits: &ObjectLimits) -> Result<ValidatedPack> {
    limits.validate()?;
    if bytes.len() > limits.pack_bytes {
        return Err(limit());
    }
    if bytes.len() < 32 || &bytes[..4] != b"PACK" {
        return Err(invalid("pack header is missing"));
    }
    let version = u32::from_be_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| invalid("pack version"))?,
    );
    if !matches!(version, 2 | 3) {
        return Err(invalid("unsupported pack version"));
    }
    let count =
        u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| invalid("pack count"))?) as usize;
    if count == 0 || count > limits.objects {
        return Err(limit());
    }
    let end = bytes.len() - 20;
    if Sha1::digest(&bytes[..end]).as_slice() != &bytes[end..] {
        return Err(invalid("pack checksum mismatch"));
    }
    let mut pos = 12usize;
    let mut entries = Vec::new();
    let mut decoded_total = 0usize;
    let mut result_total = 0usize;
    let mut offsets = BTreeMap::new();
    for index in 0..count {
        let offset = pos;
        let first = *bytes
            .get(pos)
            .filter(|_| pos < end)
            .ok_or_else(|| invalid("truncated object header"))?;
        pos += 1;
        let kind = (first >> 4) & 7;
        let mut declared = usize::from(first & 15);
        let mut last = first;
        let mut shift = 4;
        while last & 128 != 0 {
            last = *bytes
                .get(pos)
                .filter(|_| pos < end)
                .ok_or_else(|| invalid("truncated object size"))?;
            pos += 1;
            if shift >= usize::BITS || usize::from(last & 127) > (usize::MAX >> shift) {
                return Err(limit());
            }
            declared |= usize::from(last & 127) << shift;
            shift += 7;
        }
        if declared > limits.object_bytes {
            return Err(limit());
        }
        let base = match kind {
            1..=4 => Base::Direct(kind),
            6 => {
                let mut b = *bytes
                    .get(pos)
                    .filter(|_| pos < end)
                    .ok_or_else(|| invalid("truncated delta offset"))?;
                pos += 1;
                let mut distance = usize::from(b & 127);
                while b & 128 != 0 {
                    b = *bytes
                        .get(pos)
                        .filter(|_| pos < end)
                        .ok_or_else(|| invalid("truncated delta offset"))?;
                    pos += 1;
                    distance = distance
                        .checked_add(1)
                        .and_then(|v| v.checked_mul(128))
                        .and_then(|v| v.checked_add(usize::from(b & 127)))
                        .ok_or_else(limit)?;
                }
                let target = offset
                    .checked_sub(distance)
                    .filter(|_| distance > 0)
                    .ok_or_else(|| invalid("invalid delta offset"))?;
                if !offsets.contains_key(&target) {
                    return Err(invalid("delta offset is not an earlier object"));
                }
                Base::Offset(target)
            }
            7 => {
                let hash = bytes
                    .get(pos..pos.saturating_add(20))
                    .filter(|_| pos + 20 <= end)
                    .ok_or_else(|| invalid("truncated delta hash"))?
                    .try_into()
                    .map_err(|_| invalid("delta hash"))?;
                pos += 20;
                Base::Hash(hash)
            }
            _ => return Err(invalid("reserved object type")),
        };
        decoded_total = decoded_total.checked_add(declared).ok_or_else(limit)?;
        if decoded_total > limits.expanded_bytes {
            return Err(limit());
        }
        let (data, consumed) = inflate(
            bytes
                .get(pos..end)
                .ok_or_else(|| invalid("object position"))?,
            declared,
        )?;
        pos += consumed;
        let result_size = if matches!(base, Base::Direct(_)) {
            declared
        } else {
            let mut p = 0;
            let source = size(&data, &mut p)?;
            let result = size(&data, &mut p)?;
            if source > limits.object_bytes || result > limits.object_bytes {
                return Err(limit());
            }
            result
        };
        result_total = result_total.checked_add(result_size).ok_or_else(limit)?;
        if result_total > limits.expanded_bytes {
            return Err(limit());
        }
        offsets.insert(offset, index);
        entries.push(Entry { offset, base, data });
    }
    if pos != end {
        return Err(invalid("pack has trailing or undeclared object bytes"));
    }
    let mut resolved: BTreeMap<usize, Object> = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    let mut pending: Vec<usize> = (0..count).collect();
    for _ in 0..=limits.delta_depth {
        let before = pending.len();
        let mut remaining = Vec::new();
        for index in pending {
            let entry = &entries[index];
            let object = match entry.base {
                Base::Direct(kind) => Object {
                    kind,
                    data: entry.data.clone(),
                    depth: 0,
                },
                _ => {
                    let base = match &entry.base {
                        Base::Offset(offset) => resolved.get(offset),
                        Base::Hash(hash) => {
                            hashes.get(hash).and_then(|offset| resolved.get(offset))
                        }
                        _ => None,
                    };
                    let Some(base) = base else {
                        remaining.push(index);
                        continue;
                    };
                    if base.depth >= limits.delta_depth {
                        return Err(limit());
                    }
                    Object {
                        kind: base.kind,
                        data: apply_delta(&base.data, &entry.data, limits.object_bytes)?,
                        depth: base.depth + 1,
                    }
                }
            };
            let hash = object_id(object.kind, &object.data)?;
            if hashes.insert(hash, entry.offset).is_some() {
                return Err(invalid("duplicate object in pack"));
            }
            resolved.insert(entry.offset, object);
        }
        if remaining.is_empty() {
            return Ok(ValidatedPack {
                bytes,
                objects: hashes.keys().map(hex::encode).collect(),
                expanded_bytes: result_total,
            });
        }
        if remaining.len() == before {
            return Err(invalid("thin, cyclic, or unresolved delta graph"));
        }
        pending = remaining;
    }
    Err(limit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn integer(mut value: usize) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut b = (value & 127) as u8;
            value >>= 7;
            if value != 0 {
                b |= 128;
            }
            out.push(b);
            if value == 0 {
                return out;
            }
        }
    }
    fn object(kind: u8, base: Option<[u8; 20]>, data: &[u8]) -> Vec<u8> {
        let mut n = data.len();
        let mut header = vec![(kind << 4) | (n as u8 & 15)];
        n >>= 4;
        while n != 0 {
            let last = header.len() - 1;
            header[last] |= 128;
            header.push(n as u8 & 127);
            n >>= 7;
        }
        if let Some(base) = base {
            header.extend_from_slice(&base);
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        header.extend(encoder.finish().unwrap());
        header
    }
    fn archive(entries: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = b"PACK\0\0\0\x02".to_vec();
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for entry in entries {
            out.extend(entry);
        }
        let sum = Sha1::digest(&out);
        out.extend_from_slice(&sum);
        out
    }
    #[test]
    fn reconstructed_aggregate_bound_precedes_indexing_even_for_tiny_delta_programs() {
        let base = vec![b'A'; 65536];
        let mut delta = integer(base.len());
        delta.extend(integer(base.len() + 1));
        delta.extend([0x80, 1, b'B']);
        let bytes = archive(vec![
            object(3, None, &base),
            object(7, Some(object_id(3, &base).unwrap()), &delta),
        ]);
        let limits = ObjectLimits {
            expanded_bytes: base.len() + delta.len(),
            ..ObjectLimits::default()
        };
        assert_eq!(
            validate_pack(bytes.clone(), &limits).err().unwrap().code,
            "source_limit_exceeded"
        );
        let valid = validate_pack(bytes, &ObjectLimits::default()).unwrap();
        assert_eq!(valid.expanded_bytes, 131073);
        assert_eq!(valid.objects.len(), 2);
    }
    #[test]
    fn delta_depth_missing_bases_and_invalid_programs_are_rejected() {
        let mut data = b"A".to_vec();
        let mut entries = vec![object(3, None, &data)];
        for added in b"123" {
            let hash = object_id(3, &data).unwrap();
            let old = data.len();
            data.push(*added);
            let mut delta = integer(old);
            delta.extend(integer(data.len()));
            delta.push(data.len() as u8);
            delta.extend_from_slice(&data);
            entries.push(object(7, Some(hash), &delta));
        }
        let bytes = archive(entries);
        assert_eq!(
            validate_pack(
                bytes.clone(),
                &ObjectLimits {
                    delta_depth: 2,
                    ..ObjectLimits::default()
                }
            )
            .err()
            .unwrap()
            .code,
            "source_limit_exceeded"
        );
        assert_eq!(
            validate_pack(bytes, &ObjectLimits::default())
                .unwrap()
                .objects
                .len(),
            4
        );
        let missing = archive(vec![object(7, Some([0; 20]), &[1, 1, 1, b'A'])]);
        assert_eq!(
            validate_pack(missing, &ObjectLimits::default())
                .err()
                .unwrap()
                .code,
            "invalid_git_pack"
        );
        let invalid = archive(vec![
            object(3, None, b"A"),
            object(7, Some(object_id(3, b"A").unwrap()), &[1, 1, 0]),
        ]);
        assert_eq!(
            validate_pack(invalid, &ObjectLimits::default())
                .err()
                .unwrap()
                .code,
            "invalid_git_pack"
        );
    }
    #[test]
    fn strict_protocol_rejects_extra_sections_sidebands_and_trailing_packets() {
        let limits = ObjectLimits::default();
        assert!(validate_advertisement(b"000eversion 1\n0000", &limits).is_err());
        assert!(extract_pack(b"0012packfile-uris\n0000", &limits).is_err());
        assert!(extract_pack(b"000dpackfile\n0006\x04a0000", &limits).is_err());
        assert!(extract_pack(b"000dpackfile\n0000junk", &limits).is_err());
        assert!(extract_pack(b"000dpackfile\n0008\x03bad0000", &limits).is_err());
    }
}
