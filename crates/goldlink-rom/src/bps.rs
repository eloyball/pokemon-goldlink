//! BPS patches: the format byuu designed for ROM hacks. Ported from coop-launcher/src/bps.rs
//! (same author; GPL-3.0-or-later here, DECISIONS D1), which stays as it is until UC4.
//!
//! A patch is `BPS1`, the source / target / metadata sizes, the actions, then the CRC32s of the
//! source, the target and the patch itself. Actions copy from the source at the same offset
//! (SourceRead), insert bytes (TargetRead), copy from anywhere in the source (SourceCopy) or from
//! the target already written (TargetCopy, decoded but never encoded here).
//!
//! The encoder indexes the source in fixed blocks and slides a rolling hash over the target, so
//! data hg-engine's rebuild moved to another offset is still found.

const MAGIC: &[u8; 4] = b"BPS1";
const FOOTER: usize = 12;

fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

fn write_num(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let x = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(0x80 | x);
            return;
        }
        out.push(x);
        n -= 1;
    }
}

fn read_num(p: &[u8], pos: &mut usize) -> Result<u64, String> {
    let (mut data, mut shift) = (0u64, 1u64);
    loop {
        let x = *p
            .get(*pos)
            .ok_or("the patch ends in the middle of a number")?;
        *pos += 1;
        data = data
            .checked_add((x & 0x7f) as u64 * shift)
            .ok_or("a number in the patch is too large")?;
        if x & 0x80 != 0 {
            return Ok(data);
        }
        shift = shift
            .checked_shl(7)
            .ok_or("a number in the patch is too large")?;
        data = data
            .checked_add(shift)
            .ok_or("a number in the patch is too large")?;
    }
}

fn read_u32(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

/// Why a patch couldn't be applied. `WrongSource` is the one a player can fix (another ROM).
#[derive(Debug, PartialEq, Eq)]
pub enum ApplyError {
    WrongSource,
    Corrupt(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::WrongSource => write!(f, "this patch is for another ROM"),
            ApplyError::Corrupt(why) => write!(f, "the patch file is damaged ({why})"),
        }
    }
}

fn corrupt<T>(why: impl Into<String>) -> Result<T, ApplyError> {
    Err(ApplyError::Corrupt(why.into()))
}

/// Applies `patch` to `source`; checks every CRC32 the patch carries.
pub fn apply(source: &[u8], patch: &[u8]) -> Result<Vec<u8>, ApplyError> {
    if patch.len() < MAGIC.len() + FOOTER || &patch[..4] != MAGIC {
        return corrupt("not a BPS patch");
    }
    let end = patch.len() - FOOTER;
    if crc32(&patch[..patch.len() - 4]) != read_u32(patch, patch.len() - 4) {
        return corrupt("checksum");
    }
    let (source_crc, target_crc) = (read_u32(patch, end), read_u32(patch, end + 4));
    let mut pos = 4;
    let num = |pos: &mut usize| read_num(patch, pos).map_err(ApplyError::Corrupt);
    let source_size = num(&mut pos)?;
    let target_size = num(&mut pos)?;
    let metadata = num(&mut pos)? as usize;
    pos = pos
        .checked_add(metadata)
        .filter(|&p| p <= end)
        .ok_or(ApplyError::Corrupt("metadata".into()))?;
    if source.len() as u64 != source_size || crc32(source) != source_crc {
        return Err(ApplyError::WrongSource);
    }
    let target_size =
        usize::try_from(target_size).map_err(|_| ApplyError::Corrupt("target size".into()))?;
    let mut out: Vec<u8> = Vec::with_capacity(target_size);
    let (mut source_rel, mut target_rel) = (0i64, 0i64);
    while pos < end {
        let action = num(&mut pos)?;
        let length = (action >> 2) as usize + 1;
        if out.len() + length > target_size {
            return corrupt("writes past the target's end");
        }
        match action & 3 {
            0 => {
                let at = out.len();
                let bytes = source
                    .get(at..at + length)
                    .ok_or(ApplyError::Corrupt("source read".into()))?;
                out.extend_from_slice(bytes);
            }
            1 => {
                let bytes = patch.get(pos..pos + length).filter(|_| pos + length <= end);
                out.extend_from_slice(bytes.ok_or(ApplyError::Corrupt("target read".into()))?);
                pos += length;
            }
            command => {
                let d = num(&mut pos)?;
                let offset = if d & 1 != 0 {
                    -((d >> 1) as i64)
                } else {
                    (d >> 1) as i64
                };
                if command == 2 {
                    source_rel += offset;
                    let at = usize::try_from(source_rel)
                        .map_err(|_| ApplyError::Corrupt("source copy".into()))?;
                    let bytes = source
                        .get(at..at + length)
                        .ok_or(ApplyError::Corrupt("source copy".into()))?;
                    out.extend_from_slice(bytes);
                    source_rel += length as i64;
                } else {
                    target_rel += offset;
                    for _ in 0..length {
                        let at = usize::try_from(target_rel)
                            .map_err(|_| ApplyError::Corrupt("target copy".into()))?;
                        let b = *out
                            .get(at)
                            .ok_or(ApplyError::Corrupt("target copy".into()))?;
                        out.push(b);
                        target_rel += 1;
                    }
                }
            }
        }
    }
    if out.len() != target_size || crc32(&out) != target_crc {
        return corrupt("the result doesn't match");
    }
    Ok(out)
}

const BLOCK: usize = 32; // the shortest copy the encoder looks for
const TABLE_BITS: u32 = 23;
const BASE: u32 = 0x0100_0193; // rolling hash multiplier (FNV prime)

fn bucket(h: u32) -> usize {
    (h.wrapping_mul(0x9E37_79B1) >> (32 - TABLE_BITS)) as usize
}

fn hash_block(b: &[u8]) -> u32 {
    b.iter()
        .fold(0u32, |h, &x| h.wrapping_mul(BASE).wrapping_add(x as u32))
}

struct Writer {
    out: Vec<u8>,
    source_rel: u64,
}

impl Writer {
    fn literal(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            write_num(&mut self.out, ((bytes.len() as u64 - 1) << 2) | 1);
            self.out.extend_from_slice(bytes);
        }
    }

    fn copy(&mut self, source_at: usize, target_at: usize, length: usize) {
        if source_at == target_at {
            write_num(&mut self.out, (length as u64 - 1) << 2);
            return;
        }
        write_num(&mut self.out, ((length as u64 - 1) << 2) | 2);
        let offset = source_at as i64 - self.source_rel as i64;
        write_num(
            &mut self.out,
            (offset.unsigned_abs() << 1) | (offset < 0) as u64,
        );
        self.source_rel = (source_at + length) as u64;
    }
}

/// A patch that turns `source` into `target`.
pub fn create(source: &[u8], target: &[u8]) -> Vec<u8> {
    // The source's blocks at BLOCK-aligned offsets; the first one wins a bucket.
    let mut table = vec![u32::MAX; 1 << TABLE_BITS];
    for at in (0..source.len().saturating_sub(BLOCK - 1)).step_by(BLOCK) {
        let slot = &mut table[bucket(hash_block(&source[at..at + BLOCK]))];
        if *slot == u32::MAX {
            *slot = at as u32;
        }
    }
    let top = BASE.wrapping_pow(BLOCK as u32 - 1); // the leaving byte's weight
    let mut w = Writer {
        out: Vec::with_capacity(target.len() / 8),
        source_rel: 0,
    };
    w.out.extend_from_slice(MAGIC);
    write_num(&mut w.out, source.len() as u64);
    write_num(&mut w.out, target.len() as u64);
    write_num(&mut w.out, 0);

    let (mut i, mut literal_from) = (0usize, 0usize);
    let mut h = if target.len() >= BLOCK {
        hash_block(&target[..BLOCK])
    } else {
        0
    };
    while i + BLOCK <= target.len() {
        // The same offset first (unchanged data), then anywhere through the index.
        let found = if i + BLOCK <= source.len() && source[i..i + BLOCK] == target[i..i + BLOCK] {
            Some(i)
        } else {
            let at = table[bucket(h)];
            (at != u32::MAX && source[at as usize..at as usize + BLOCK] == target[i..i + BLOCK])
                .then_some(at as usize)
        };
        let Some(mut s) = found else {
            if i + BLOCK < target.len() {
                h = h
                    .wrapping_sub((target[i] as u32).wrapping_mul(top))
                    .wrapping_mul(BASE)
                    .wrapping_add(target[i + BLOCK] as u32);
            }
            i += 1;
            continue;
        };
        // Grow the match backwards into the pending literal, then forwards.
        let mut t = i;
        while t > literal_from && s > 0 && source[s - 1] == target[t - 1] {
            s -= 1;
            t -= 1;
        }
        let mut length = i - t + BLOCK;
        while s + length < source.len()
            && t + length < target.len()
            && source[s + length] == target[t + length]
        {
            length += 1;
        }
        w.literal(&target[literal_from..t]);
        w.copy(s, t, length);
        i = t + length;
        literal_from = i;
        if i + BLOCK <= target.len() {
            h = hash_block(&target[i..i + BLOCK]);
        }
    }
    w.literal(&target[literal_from..]);

    let mut out = w.out;
    out.extend_from_slice(&crc32(source).to_le_bytes());
    out.extend_from_slice(&crc32(target).to_le_bytes());
    let own = crc32(&out);
    out.extend_from_slice(&own.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    #[test]
    fn numbers_round_trip() {
        for n in [
            0u64,
            1,
            127,
            128,
            129,
            16383,
            16384,
            1 << 40,
            u32::MAX as u64,
        ] {
            let mut v = Vec::new();
            write_num(&mut v, n);
            let mut pos = 0;
            assert_eq!(read_num(&v, &mut pos).unwrap(), n);
            assert_eq!(pos, v.len());
        }
    }

    #[test]
    fn moved_inserted_and_changed_data() {
        let source = noise(200_000, 7);
        let mut target = Vec::new();
        target.extend_from_slice(&source[50_000..120_000]); // moved
        target.extend_from_slice(&noise(3_000, 99)); // new
        target.extend_from_slice(&source[..50_000]); // moved back
        target.extend_from_slice(&source[120_000..]);
        target[10] ^= 0xFF; // changed in place
        target.extend(std::iter::repeat_n(0xFF, 10_000)); // padding
        let patch = create(&source, &target);
        assert!(patch.len() < 20_000, "patch {} bytes", patch.len());
        assert_eq!(apply(&source, &patch).unwrap(), target);
    }

    #[test]
    fn identical_and_empty() {
        let source = noise(5_000, 3);
        assert_eq!(apply(&source, &create(&source, &source)).unwrap(), source);
        assert_eq!(
            apply(&source, &create(&source, &[])).unwrap(),
            Vec::<u8>::new()
        );
        assert_eq!(apply(&[], &create(&[], &source)).unwrap(), source);
    }

    #[test]
    fn wrong_source_and_damage() {
        let source = noise(10_000, 1);
        let mut target = source.clone();
        target[500] = 0;
        let patch = create(&source, &target);
        let mut other = source.clone();
        other[0] ^= 1;
        assert_eq!(apply(&other, &patch), Err(ApplyError::WrongSource));
        let mut bad = patch.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x40;
        assert!(matches!(apply(&source, &bad), Err(ApplyError::Corrupt(_))));
    }

    #[test]
    fn target_copy_decodes() {
        // A hand-written patch: "ab" (TargetRead), then TargetCopy of 4 bytes from offset 0: "ababab".
        let source: Vec<u8> = Vec::new();
        let target = b"ababab".to_vec();
        let mut p = MAGIC.to_vec();
        write_num(&mut p, 0);
        write_num(&mut p, 6);
        write_num(&mut p, 0);
        write_num(&mut p, (1 << 2) | 1);
        p.extend_from_slice(b"ab");
        write_num(&mut p, (3 << 2) | 3);
        write_num(&mut p, 0);
        p.extend_from_slice(&crc32(&source).to_le_bytes());
        p.extend_from_slice(&crc32(&target).to_le_bytes());
        let own = crc32(&p);
        p.extend_from_slice(&own.to_le_bytes());
        assert_eq!(apply(&source, &p).unwrap(), target);
    }
}
