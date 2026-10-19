//! Damaged files give errors, not panics or huge allocations, and a model.dat
//! deflated by another encoder reads the same as SketchUp's own.

use crate::{face_count, fixture, largest_allocation, read, ALLOCATION_LIMIT, FIXTURES};
use skp::container::Container;
use skp::source::{ByteSource, MemorySource};
use skp::stream::{InflateRead, SeqRead};
use skp::zip::ZipDirectory;
use std::io::{Cursor, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// A small deterministic generator, so a failing mutant can be reproduced.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Where the zip central directory sits, from the end-of-directory record.
fn central_directory(bytes: &[u8]) -> std::ops::Range<usize> {
    let eocd = bytes
        .windows(4)
        .rposition(|w| w == b"PK\x05\x06")
        .expect("EOCD");
    let size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    eocd - size..eocd
}

/// 200 damaged copies of `bytes`, each with a label.
fn mutants(bytes: &[u8], seed: u64) -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    let len = bytes.len();
    for k in 0..20 {
        let at = len * k / 20 + rng.below(len / 20);
        out.push((format!("truncated at {at}"), bytes[..at].to_vec()));
    }
    for (flips, count) in [(1, 60), (10, 60), (100, 50)] {
        for k in 0..count {
            let mut m = bytes.to_vec();
            for _ in 0..flips {
                let at = rng.below(len);
                m[at] ^= 1 << rng.below(8);
            }
            out.push((format!("{flips} bits flipped, mutant {k}"), m));
        }
    }
    for k in 0..9 {
        let mut m = bytes.to_vec();
        let at = rng.below(len - 4096);
        m[at..at + 4096].fill(0);
        out.push((format!("4 KiB zeroed at {at}, mutant {k}"), m));
    }
    let mut m = bytes.to_vec();
    m[central_directory(bytes)].fill(0xFF);
    out.push(("central directory overwritten".to_string(), m));
    assert_eq!(out.len(), 200);
    out
}

/// Everything an importer reads: the model, its index and every zip entry.
fn read_everything(bytes: Vec<u8>) {
    if let Ok((c, m, src)) = read(bytes) {
        let _ = m.index(&src, &c);
        for e in c.zip.iter().flat_map(|z| &z.entries) {
            let _ = c.read(&src, e);
        }
    }
}

#[test]
fn mutated_fixtures_never_panic() {
    let mut failed = Vec::new();
    for (seed, name) in FIXTURES.into_iter().enumerate() {
        for (label, m) in mutants(&fixture(name), 0x9E37_79B9_7F4A_7C15 + seed as u64) {
            let mut outcome = Ok(());
            let largest = largest_allocation(|| {
                outcome = catch_unwind(AssertUnwindSafe(|| read_everything(m)));
            });
            if outcome.is_err() {
                failed.push(format!("{name}: {label}: panicked"));
            } else if largest > ALLOCATION_LIMIT {
                failed.push(format!(
                    "{name}: {label}: allocated {largest} bytes at once"
                ));
            }
        }
    }
    assert!(failed.is_empty(), "failed:\n{}", failed.join("\n"));
}

#[test]
fn huge_lengths_are_refused_before_allocating() {
    for name in FIXTURES {
        let mut bytes = fixture(name);
        let cd = central_directory(&bytes);
        let at = cd.start
            + bytes[cd.clone()]
                .windows(4)
                .position(|w| w == b"PK\x01\x02")
                .expect("first central directory entry");
        bytes[at + 20..at + 24].copy_from_slice(&u32::MAX.to_le_bytes());

        let src = MemorySource::new(bytes.clone());
        let c = Container::open(&src).expect(name);
        let entry = &c.zip.as_ref().expect("zip").entries[0];
        assert_eq!(entry.compressed_size, u64::from(u32::MAX), "{name}");
        let largest = largest_allocation(|| {
            assert!(c.read(&src, entry).is_err(), "{name}: {} read", entry.name);
        });
        assert!(
            largest < ALLOCATION_LIMIT,
            "{name}: entry read allocated {largest}"
        );

        let mut model = fixture(name);
        let cd = central_directory(&model);
        let dir = ZipDirectory::find(&MemorySource::new(model.clone())).expect(name);
        let index = dir
            .entries
            .iter()
            .position(|e| e.name == "model.dat")
            .expect("model.dat");
        let at = nth_central_entry(&model[cd.clone()], index) + cd.start;
        model[at + 20..at + 24].copy_from_slice(&u32::MAX.to_le_bytes());
        let largest = largest_allocation(|| {
            assert!(
                read(model).is_err(),
                "{name}: model.dat with a 4 GiB size read"
            );
        });
        assert!(
            largest < ALLOCATION_LIMIT,
            "{name}: model read allocated {largest}"
        );

        // The directory's own size, claimed as 4 GiB.
        let mut directory = fixture(name);
        let eocd = directory
            .windows(4)
            .rposition(|w| w == b"PK\x05\x06")
            .expect("EOCD");
        directory[eocd + 12..eocd + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        let largest = largest_allocation(|| {
            assert!(read(directory).is_err(), "{name}: a 4 GiB directory read");
        });
        assert!(
            largest < ALLOCATION_LIMIT,
            "{name}: directory read allocated {largest}"
        );
    }
}

/// Offset of the `n`th central directory record within `cd`.
fn nth_central_entry(cd: &[u8], n: usize) -> usize {
    let mut at = 0;
    for _ in 0..n {
        let le = |o: usize| u16::from_le_bytes([cd[at + o], cd[at + o + 1]]) as usize;
        at += 46 + le(28) + le(30) + le(32);
    }
    assert_eq!(&cd[at..at + 4], b"PK\x01\x02");
    at
}

/// The fixture with `model.dat` deflated again by miniz_oxide at level 9 and
/// every other entry stored, behind the original SketchUp header.
fn rezipped(bytes: &[u8]) -> Vec<u8> {
    rebuilt(bytes, |_| {}, &[])
}

/// An entry added to a rebuilt fixture: its name, its deflated bytes and the
/// inflated size its headers claim.
struct Extra<'a> {
    name: &'a str,
    deflated: &'a [u8],
    claimed: u32,
}

/// The fixture rebuilt: `edit` applied to the inflated `model.dat`, which is
/// then deflated at level 9; every other entry stored; `extra` entries
/// appended; the original SketchUp header in front.
fn rebuilt(bytes: &[u8], edit: impl FnOnce(&mut Vec<u8>), extra: &[Extra]) -> Vec<u8> {
    let src = MemorySource::new(bytes.to_vec());
    let c = Container::open(&src).expect("open");
    let header = bytes
        .windows(4)
        .position(|w| w == b"PK\x03\x04")
        .expect("zip start");

    let mut cursor = Cursor::new(bytes[..header].to_vec());
    cursor.set_position(header as u64);
    let mut zip = zip::ZipWriter::new(cursor);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    // (name, inflated size) of each entry written deflated but marked stored.
    let mut deflated: Vec<(String, u32)> = Vec::new();
    let mut edit = Some(edit);
    for e in c.zip.as_ref().expect("zip").entries.iter() {
        let mut data = c.read(&src, e).expect("entry");
        zip.start_file(e.name.as_str(), stored)
            .expect("start entry");
        if e.name == "model.dat" {
            (edit.take().expect("one model.dat"))(&mut data);
            deflated.push((e.name.clone(), data.len() as u32));
            zip.write_all(&miniz_oxide::deflate::compress_to_vec(&data, 9))
                .expect("write");
        } else {
            zip.write_all(&data).expect("write");
        }
    }
    for x in extra {
        zip.start_file(x.name, stored).expect("start entry");
        zip.write_all(x.deflated).expect("write");
        deflated.push((x.name.to_string(), x.claimed));
    }
    let mut out = zip.finish().expect("finish").into_inner();

    // Those entries were written stored; mark them deflated with their
    // inflated size, in both the local header and the central directory.
    let dir = ZipDirectory::find(&MemorySource::new(out.clone())).expect("new directory");
    for (name, size) in deflated {
        let index = dir
            .entries
            .iter()
            .position(|e| e.name == name)
            .expect("entry");
        let local = dir.entries[index].header_offset as usize;
        let cd = central_directory(&out);
        let central = cd.start + nth_central_entry(&out[cd], index);
        for (at, size_at) in [(local + 8, local + 22), (central + 10, central + 24)] {
            out[at..at + 2].copy_from_slice(&8u16.to_le_bytes());
            out[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        }
    }
    out
}

#[test]
fn rezipped_model_reads() {
    for name in FIXTURES {
        let bytes = fixture(name);
        let (_, original, _) = read(bytes.clone()).expect(name);
        let (_, again, _) = read(rezipped(&bytes)).expect(name);
        assert_eq!(face_count(&again), face_count(&original), "{name}");
    }
}

/// Every byte of a deflate stream comes out, including what the inflater
/// still holds once the input is used up. The fixture's model.dat repeated
/// and deflated at a few levels gives streams of several input chunks; under
/// the original reader the desk repeated four times at level 1 lost its last
/// 11,941 bytes.
#[test]
fn inflate_reads_every_byte() {
    for name in FIXTURES {
        let src = MemorySource::new(fixture(name));
        let c = Container::open(&src).expect(name);
        let model = c
            .read(&src, c.model_entry().expect("model.dat"))
            .expect(name);
        for copies in 1..=4 {
            let data = model.repeat(copies);
            for level in [1, 9] {
                let deflated = miniz_oxide::deflate::compress_to_vec(&data, level);
                let src = MemorySource::new(deflated);
                let mut r =
                    InflateRead::new(&src, 0, src.len(), data.len() as u64).expect("in range");
                let mut out = Vec::with_capacity(data.len());
                let mut byte = [0u8; 1];
                while r.read_exact(&mut byte).is_ok() {
                    out.push(byte[0]);
                }
                assert!(
                    out == data,
                    "{name} x{copies} at level {level}: {} of {} bytes",
                    out.len(),
                    data.len()
                );
            }
        }
    }
}

/// A `material.xml` that inflates to far more than it is: 64 MiB of zeros in
/// about 64 KiB. Claiming 4 GiB it is refused outright; claiming its true size
/// it is still more than a material description can need, and the index
/// skips it. Neither read allocates more than the limit.
#[test]
fn inflated_entries_are_capped() {
    let zeros = miniz_oxide::deflate::compress_to_vec(&vec![0u8; 64 << 20], 1);
    for name in FIXTURES {
        for claimed in [u32::MAX, 64 << 20] {
            let extra = Extra {
                name: "materials/Bomb/material.xml",
                deflated: &zeros,
                claimed,
            };
            let bytes = rebuilt(&fixture(name), |_| {}, &[extra]);
            let (c, m, src) = read(bytes).expect(name);
            let entry = c
                .zip
                .as_ref()
                .expect("zip")
                .entries
                .iter()
                .find(|e| e.name == "materials/Bomb/material.xml");
            let entry = entry.expect("added entry");
            let largest = largest_allocation(|| {
                let _ = m.index(&src, &c);
            });
            assert!(
                largest < ALLOCATION_LIMIT,
                "{name} claiming {claimed}: index allocated {largest}"
            );
            if claimed == u32::MAX {
                let largest = largest_allocation(|| {
                    assert!(c.read(&src, entry).is_err(), "{name}: a 4 GiB entry read");
                });
                assert!(
                    largest < ALLOCATION_LIMIT,
                    "{name}: entry read allocated {largest}"
                );
            }
        }
    }
}

/// A record in `model.dat` claiming 700 MiB, under the 768 MiB sanity cap, on
/// a stream that holds well under 1 MiB: the read fails without allocating
/// the claim.
#[test]
fn record_sizes_are_not_allocated_up_front() {
    use skp::tags::{OVERFLOW_SIZE_TAG, SKETCH_UP_MODEL_ROOT};
    for name in FIXTURES {
        let bytes = rebuilt(
            &fixture(name),
            |model| {
                let u16_at = |at: usize| u16::from_le_bytes([model[at], model[at + 1]]);
                let u32_at = |at: usize| u32::from_le_bytes(model[at..at + 4].try_into().unwrap());
                // The model record's header, then its children.
                let mut at = 6;
                if u16_at(at) == OVERFLOW_SIZE_TAG {
                    at += 10;
                }
                while u16_at(at) != SKETCH_UP_MODEL_ROOT {
                    at += 6 + u32_at(at + 2) as usize;
                }
                model[at + 2..at + 6].copy_from_slice(&(700u32 << 20).to_le_bytes());
            },
            &[],
        );
        let largest = largest_allocation(|| {
            assert!(read(bytes).is_err(), "{name}: a 700 MiB record read");
        });
        assert!(largest < ALLOCATION_LIMIT, "{name}: allocated {largest}");
    }
}
