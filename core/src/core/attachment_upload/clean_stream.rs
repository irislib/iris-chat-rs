//! Bounded-memory photo/video cleanup.
//!
//! A cleaner first parses the file through `Source`, keeping only small records
//! (headers, kept segments) in a `Plan` and referring to bulk media by offset.
//! The plan is then streamed into the upload store. A block the parser read is
//! hashed before it leaves the cache, and the stream refuses a reread block
//! that changed since, so
//! a file rewritten mid-upload cannot slip unchecked bytes past the cleaner.
//! Bytes the parser never inspected cannot affect its decisions.

use hashtree_core::Hash;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};

const BLOCK: usize = 64 * 1024;
const CACHED_BLOCKS: usize = 4;

pub(super) trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

pub(super) struct Source {
    len: usize,
    state: RefCell<SourceState>,
}

struct SourceState {
    reader: Box<dyn ReadSeek>,
    cache: VecDeque<(usize, Vec<u8>)>,
    hashes: Vec<Option<Hash>>,
}

impl Source {
    pub(super) fn new(reader: impl ReadSeek + 'static, len: usize) -> Self {
        Self {
            len,
            state: RefCell::new(SourceState {
                reader: Box::new(reader),
                cache: VecDeque::with_capacity(CACHED_BLOCKS),
                hashes: vec![None; len.div_ceil(BLOCK)],
            }),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// Fill `buf` from `at`. `None` when out of bounds, unreadable, or changed.
    pub(super) fn read_into(&self, mut at: usize, mut buf: &mut [u8]) -> Option<()> {
        if at.checked_add(buf.len())? > self.len {
            return None;
        }
        while !buf.is_empty() {
            let offset = at % BLOCK;
            let n = buf.len().min(BLOCK - offset);
            let (dest, rest) = buf.split_at_mut(n);
            self.with_block(at / BLOCK, |block| {
                dest.copy_from_slice(block.get(offset..offset + n)?);
                Some(())
            })?;
            buf = rest;
            at += n;
        }
        Some(())
    }

    pub(super) fn read(&self, at: usize, len: usize) -> Option<Vec<u8>> {
        if at.checked_add(len)? > self.len {
            return None;
        }
        let mut out = vec![0; len];
        self.read_into(at, &mut out)?;
        Some(out)
    }

    pub(super) fn byte(&self, at: usize) -> Option<u8> {
        if at >= self.len {
            return None;
        }
        self.with_block(at / BLOCK, |block| block.get(at % BLOCK).copied())
    }

    fn with_block<T>(&self, index: usize, f: impl FnOnce(&[u8]) -> Option<T>) -> Option<T> {
        let mut state = self.state.borrow_mut();
        if let Some((_, block)) = state.cache.iter().find(|(i, _)| *i == index) {
            return f(block);
        }
        let start = index.checked_mul(BLOCK)?;
        let mut block = vec![0; BLOCK.min(self.len.checked_sub(start)?)];
        state.reader.seek(SeekFrom::Start(start as u64)).ok()?;
        state.reader.read_exact(&mut block).ok()?;
        // A block is hashed when it leaves the cache, so only a reread pays.
        if let Some(seen) = state.hashes.get(index)? {
            if *seen != hashtree_core::sha256(&block) {
                return None;
            }
        }
        if state.cache.len() == CACHED_BLOCKS {
            let (evicted, bytes) = state.cache.pop_front()?;
            let slot = state.hashes.get_mut(evicted)?;
            if slot.is_none() {
                *slot = Some(hashtree_core::sha256(&bytes));
            }
        }
        let value = f(&block);
        state.cache.push_back((index, block));
        value
    }
}

enum Piece {
    Bytes(Vec<u8>),
    Copy(usize, usize),
    Zero(usize),
}

impl Piece {
    fn len(&self) -> usize {
        match self {
            Piece::Bytes(bytes) => bytes.len(),
            Piece::Copy(_, len) | Piece::Zero(len) => *len,
        }
    }
}

/// Cleaned output: small rewritten records plus source ranges to stream.
#[derive(Default)]
pub(super) struct Plan {
    pieces: Vec<Piece>,
    len: usize,
}

impl Plan {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.len += bytes.len();
        if let Some(Piece::Bytes(last)) = self.pieces.last_mut() {
            last.extend_from_slice(bytes);
        } else {
            self.pieces.push(Piece::Bytes(bytes.to_vec()));
        }
    }

    pub(super) fn copy(&mut self, at: usize, len: usize) {
        if len == 0 {
            return;
        }
        self.len += len;
        if let Some(Piece::Copy(start, last)) = self.pieces.last_mut() {
            if *start + *last == at {
                *last += len;
                return;
            }
        }
        self.pieces.push(Piece::Copy(at, len));
    }

    pub(super) fn zero(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.len += len;
        if let Some(Piece::Zero(last)) = self.pieces.last_mut() {
            *last += len;
        } else {
            self.pieces.push(Piece::Zero(len));
        }
    }

    /// Mutable access to already pushed literal bytes at output position `at`.
    pub(super) fn literal_mut(&mut self, mut at: usize, len: usize) -> Option<&mut [u8]> {
        for piece in &mut self.pieces {
            let piece_len = piece.len();
            if at < piece_len {
                return match piece {
                    Piece::Bytes(bytes) => bytes.get_mut(at..at.checked_add(len)?),
                    _ => None,
                };
            }
            at -= piece_len;
        }
        None
    }
}

pub(super) struct PlanReader {
    source: Source,
    pieces: std::vec::IntoIter<Piece>,
    current: Option<Piece>,
    done: usize,
}

impl PlanReader {
    pub(super) fn new(source: Source, plan: Plan) -> Self {
        Self {
            source,
            pieces: plan.pieces.into_iter(),
            current: None,
            done: 0,
        }
    }
}

impl Read for PlanReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let Some(piece) = &self.current else {
                self.current = self.pieces.next();
                self.done = 0;
                if self.current.is_none() {
                    return Ok(0);
                }
                continue;
            };
            let n = buf.len().min(piece.len() - self.done);
            if n == 0 {
                if buf.is_empty() {
                    return Ok(0);
                }
                self.current = None;
                continue;
            }
            let (dest, _) = buf.split_at_mut(n);
            match piece {
                Piece::Bytes(bytes) => {
                    let chunk = bytes
                        .get(self.done..self.done + n)
                        .ok_or_else(|| io::Error::other("invalid attachment literal range"))?;
                    dest.copy_from_slice(chunk);
                }
                Piece::Zero(_) => dest.fill(0),
                Piece::Copy(at, _) => self
                    .source
                    .read_into(
                        at.checked_add(self.done)
                            .ok_or_else(|| io::Error::other("invalid attachment source range"))?,
                        dest,
                    )
                    .ok_or_else(|| io::Error::other("attachment changed while uploading"))?,
            }
            self.done += n;
            return Ok(n);
        }
    }
}

/// Stream a plan into an async pipe. Run on a blocking thread.
pub(super) fn write_to(
    source: Source,
    plan: Plan,
    mut pipe: tokio::io::DuplexStream,
    runtime: tokio::runtime::Handle,
) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut reader = PlanReader::new(source, plan);
    let mut buf = vec![0; BLOCK];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return runtime.block_on(pipe.shutdown());
        }
        let chunk = buf
            .get(..n)
            .ok_or_else(|| io::Error::other("invalid attachment read length"))?;
        runtime.block_on(pipe.write_all(chunk))?;
    }
}

#[cfg(test)]
pub(super) fn run<E>(
    data: &[u8],
    plan: impl FnOnce(&Source) -> Result<Plan, E>,
) -> Result<Vec<u8>, E> {
    let source = Source::new(io::Cursor::new(data.to_vec()), data.len());
    let plan = plan(&source)?;
    let expected = plan.len();
    let mut out = Vec::new();
    PlanReader::new(source, plan).read_to_end(&mut out).unwrap();
    assert_eq!(out.len(), expected);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    struct Shared(Arc<Mutex<Vec<u8>>>, u64);

    impl Read for Shared {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let data = self.0.lock().unwrap();
            let n = (&data[self.1 as usize..]).read(buf)?;
            self.1 += n as u64;
            Ok(n)
        }
    }

    impl Seek for Shared {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            let SeekFrom::Start(at) = pos else {
                unreachable!()
            };
            self.1 = at;
            Ok(at)
        }
    }

    #[test]
    fn streams_literals_zeros_and_copies_across_blocks() {
        let data: Vec<u8> = (0..BLOCK * 3 + 17).map(|i| i as u8).collect();
        let out = run(&data, |source| {
            let mut plan = Plan::default();
            plan.push(b"head");
            plan.copy(BLOCK - 3, 10);
            plan.copy(BLOCK + 7, BLOCK * 2);
            plan.zero(5);
            plan.literal_mut(1, 2).unwrap().copy_from_slice(b"EA");
            assert_eq!(source.byte(source.len() - 1), Some(data[data.len() - 1]));
            Ok::<_, ()>(plan)
        })
        .unwrap();
        let mut expected = b"hEAd".to_vec();
        expected.extend_from_slice(&data[BLOCK - 3..BLOCK + 7]);
        expected.extend_from_slice(&data[BLOCK + 7..BLOCK * 3 + 7]);
        expected.extend_from_slice(&[0; 5]);
        assert_eq!(out, expected);
    }

    #[test]
    fn refuses_an_inspected_block_that_changed_before_streaming() {
        let data = Arc::new(Mutex::new(vec![7u8; BLOCK * (CACHED_BLOCKS + 1)]));
        let len = data.lock().unwrap().len();
        let source = Source::new(Shared(data.clone(), 0), len);
        assert_eq!(source.byte(3), Some(7));
        // Evict the inspected block so streaming must read it again.
        for index in 1..=CACHED_BLOCKS {
            source.byte(index * BLOCK).unwrap();
        }
        data.lock().unwrap()[BLOCK - 1] = 8;
        let mut plan = Plan::default();
        plan.copy(0, 8);
        let error = PlanReader::new(source, plan)
            .read_to_end(&mut Vec::new())
            .unwrap_err();
        assert_eq!(error.to_string(), "attachment changed while uploading");
    }
}
