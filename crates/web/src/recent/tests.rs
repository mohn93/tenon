use std::cell::RefCell;
use std::rc::Rc;

use super::*;

/// A storage in memory, writes done at once, that notes what was asked of it.
#[derive(Clone, Default)]
struct Fake {
    records: Rc<RefCell<Vec<Record>>>,
    blocked: bool,
    /// Writes fail (storage full).
    full: Rc<std::cell::Cell<bool>>,
    failed: Rc<std::cell::Cell<bool>>,
}

impl Storage for Fake {
    fn available(&self) -> bool {
        !self.blocked
    }

    fn infos(&self) -> Vec<Info> {
        self.records
            .borrow()
            .iter()
            .map(|r| r.info.clone())
            .collect()
    }

    fn put(&mut self, record: Record, then_remove: Vec<String>) {
        if self.full.get() {
            self.failed.set(true);
            return;
        }
        self.failed.set(false);
        let mut all = self.records.borrow_mut();
        all.retain(|r| r.info.id != record.info.id && !then_remove.contains(&r.info.id));
        all.push(record);
    }

    fn write_failed(&self) -> bool {
        self.failed.get()
    }

    fn clear_failure(&mut self) {
        self.failed.set(false);
    }

    fn touch(&mut self, id: &str, time: f64) {
        for r in self
            .records
            .borrow_mut()
            .iter_mut()
            .filter(|r| r.info.id == id)
        {
            r.info.time = time;
        }
    }

    fn remove(&mut self, id: &str) {
        self.records.borrow_mut().retain(|r| r.info.id != id);
    }

    fn get(&self, id: &str, done: Box<dyn FnOnce(Option<Record>)>) {
        let found = self
            .records
            .borrow()
            .iter()
            .find(|r| r.info.id == id)
            .cloned();
        done(found);
    }
}

fn recent() -> (Recent, Fake) {
    let fake = Fake::default();
    (Recent::new(Box::new(fake.clone())), fake)
}

fn id(n: u8) -> String {
    id_hex(&[n; 16])
}

fn record(n: u8, time: f64, size: usize) -> Record {
    Record {
        info: Info {
            id: id(n),
            name: format!("Model {n}"),
            time,
            size,
        },
        key: [n; 32],
        blob: vec![n; size],
    }
}

fn ids(r: &Recent) -> Vec<String> {
    r.list(1e9).into_iter().map(|e| e.id).collect()
}

#[test]
fn the_list_is_newest_first() {
    let (mut r, _) = recent();
    for (n, t) in [(1, 100.0), (3, 300.0), (2, 200.0)] {
        assert_eq!(r.keep(record(n, t, 10)), Kept::Yes);
    }
    assert_eq!(ids(&r), [id(3), id(2), id(1)]);
    let names: Vec<_> = r.list(1e9).into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["Model 3", "Model 2", "Model 1"]);
}

#[test]
fn only_the_last_five_are_kept_and_the_oldest_go() {
    let (mut r, fake) = recent();
    for n in 1..=8u8 {
        r.keep(record(n, f64::from(n) * 10.0, 10));
        assert!(r.list(1e9).len() <= KEEP);
    }
    assert_eq!(ids(&r), [id(8), id(7), id(6), id(5), id(4)]);
    assert_eq!(
        fake.records.borrow().len(),
        5,
        "the storage dropped them too"
    );
}

#[test]
fn opening_a_share_again_replaces_its_entry_instead_of_adding_one() {
    let (mut r, _) = recent();
    for n in 1..=5u8 {
        r.keep(record(n, f64::from(n), 10));
    }
    r.keep(record(1, 99.0, 10));
    assert_eq!(r.list(1e9).len(), 5);
    assert_eq!(ids(&r)[0], id(1), "now the newest");
}

#[test]
fn a_share_over_64_mib_is_not_kept_and_the_list_says_why() {
    let (mut r, fake) = recent();
    r.keep(record(1, 1.0, 10));
    let big = Record {
        info: Info {
            id: id(2),
            name: "Big".into(),
            time: 2.0,
            size: MAX_KEPT_BYTES + 1,
        },
        key: [2; 32],
        blob: vec![0; MAX_KEPT_BYTES + 1],
    };
    assert_eq!(r.keep(big), Kept::TooLarge);
    assert_eq!(r.note(), Some(TOO_LARGE));
    assert_eq!(ids(&r), [id(1)], "nothing was added or dropped");
    assert_eq!(fake.records.borrow().len(), 1);
    // Exactly 64 MiB is kept, and the note goes.
    let exact = Record {
        info: Info {
            id: id(3),
            name: "Exact".into(),
            time: 3.0,
            size: MAX_KEPT_BYTES,
        },
        key: [3; 32],
        blob: vec![0; MAX_KEPT_BYTES],
    };
    assert_eq!(r.keep(exact), Kept::Yes);
    assert_eq!(r.note(), None);
    assert_eq!(r.list(1e9).len(), 2);
}

#[test]
fn delete_removes_one_and_only_that_one() {
    let (mut r, _) = recent();
    for n in 1..=3u8 {
        r.keep(record(n, f64::from(n), 10));
    }
    r.delete(&id(2));
    assert_eq!(ids(&r), [id(3), id(1)]);
    r.delete(&id(2));
    r.delete("nonsense");
    assert_eq!(ids(&r), [id(3), id(1)]);
}

#[test]
fn opening_reads_the_record_back_and_makes_it_the_newest() {
    let (mut r, _) = recent();
    for n in 1..=3u8 {
        r.keep(record(n, f64::from(n), 10));
    }
    let got = Rc::new(RefCell::new(None));
    let sink = got.clone();
    r.open(
        &id(1),
        50.0,
        Box::new(move |rec| *sink.borrow_mut() = Some(rec)),
    );
    let rec = got.borrow_mut().take().unwrap().expect("found");
    assert_eq!(rec.info.name, "Model 1");
    assert_eq!(rec.key, [1; 32]);
    assert_eq!(ids(&r)[0], id(1));
    // A kept model that was opened after one too big to keep: the note is about the old one.
    let big = Record {
        info: Info {
            id: id(8),
            name: "Big".into(),
            time: 70.0,
            size: MAX_KEPT_BYTES + 1,
        },
        key: [8; 32],
        blob: vec![0; MAX_KEPT_BYTES + 1],
    };
    assert_eq!(r.keep(big), Kept::TooLarge);
    assert!(r.note().is_some());
    r.open(&id(2), 80.0, Box::new(|_| {}));
    assert_eq!(r.note(), None);
    // A missing one answers None and changes nothing.
    r.open(
        &id(9),
        60.0,
        Box::new(move |rec| *got.borrow_mut() = Some(rec)),
    );
    assert_eq!(ids(&r).len(), 3);
}

#[test]
fn a_browser_that_keeps_nothing_says_so_and_keeps_nothing() {
    let fake = Fake {
        blocked: true,
        ..Fake::default()
    };
    let mut r = Recent::new(Box::new(fake.clone()));
    assert_eq!(r.note(), Some(NO_STORAGE));
    assert_eq!(r.keep(record(1, 1.0, 10)), Kept::NoStorage);
    assert!(fake.records.borrow().is_empty());
    assert!(r.list(0.0).is_empty());
}

#[test]
fn what_is_kept_decrypts_offline_with_its_own_key() {
    let model = b"the model bytes".to_vec();
    let sealed = session::share::seal("Kitchen", &model).unwrap();
    let rec = Record {
        info: Info {
            id: id_hex(&sealed.id),
            name: "Kitchen".into(),
            time: 1.0,
            size: sealed.blob.len(),
        },
        key: sealed.key,
        blob: sealed.blob.clone(),
    };
    assert_eq!(unseal(&rec).unwrap(), ("Kitchen".to_string(), model));
    // A damaged copy, or one under another id, is refused.
    let mut bad = rec.clone();
    bad.blob[20] ^= 1;
    assert!(unseal(&bad).unwrap_err().contains("damaged"));
    let mut other = rec;
    other.info.id = id(7);
    assert!(unseal(&other).is_err());
    let mut junk = other;
    junk.info.id = "xyz".into();
    assert!(unseal(&junk).is_err());
}

#[test]
fn ids_round_trip_through_hex() {
    let raw = [
        0u8, 1, 0xab, 0xff, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 0x7f,
    ];
    let hex = id_hex(&raw);
    assert_eq!(hex.len(), 32);
    assert_eq!(parse_id(&hex), Some(raw));
    for bad in [
        "",
        "ab",
        &hex.to_uppercase(),
        &hex[..31],
        &format!("{hex}0"),
        &"g".repeat(32),
    ] {
        assert_eq!(parse_id(bad), None, "{bad}");
    }
}

#[test]
fn the_key_is_never_shown_by_debug() {
    let rec = record(4, 1.0, 3);
    let text = format!("{rec:?}");
    assert!(!text.contains("[4, 4"), "{text}");
    assert!(text.contains("<hidden>"));
}

#[test]
fn ages_are_written_in_coarse_units() {
    let min = 60_000.0;
    assert_eq!(age_text(1000.0, 1000.0), "just now");
    assert_eq!(age_text(59.0 * 1000.0, 0.0), "just now");
    assert_eq!(age_text(min, 0.0), "1 min ago");
    assert_eq!(age_text(59.0 * min, 0.0), "59 min ago");
    assert_eq!(age_text(60.0 * min, 0.0), "1 h ago");
    assert_eq!(age_text(23.0 * 60.0 * min, 0.0), "23 h ago");
    assert_eq!(age_text(24.0 * 60.0 * min, 0.0), "1 day ago");
    assert_eq!(age_text(5.0 * 24.0 * 60.0 * min, 0.0), "5 days ago");
    assert_eq!(
        age_text(0.0, 10.0 * min),
        "just now",
        "a clock that moved back"
    );
}

#[test]
fn sizes_are_written_in_bytes_kilobytes_and_megabytes() {
    assert_eq!(size_text(0), "0 B");
    assert_eq!(size_text(1023), "1023 B");
    assert_eq!(size_text(1024), "1 KB");
    assert_eq!(size_text(812 * 1024), "812 KB");
    assert_eq!(size_text(1024 * 1024), "1.0 MB");
    assert_eq!(size_text(12_900_000), "12.3 MB");
}

#[test]
fn the_database_and_store_names_are_the_documented_ones() {
    assert_eq!(DB_NAME, "tenon-viewer");
    assert_eq!(STORE, "recent");
    assert_eq!(KEEP, 5);
    assert_eq!(MAX_KEPT_BYTES, 64 * 1024 * 1024);
}

#[test]
fn a_too_large_note_set_while_waiting_survives_the_storage_arriving() {
    let big = |n: u8| Record {
        info: Info {
            id: id(n),
            name: "Big".into(),
            time: 1.0,
            size: MAX_KEPT_BYTES + 1,
        },
        key: [n; 32],
        blob: vec![0; MAX_KEPT_BYTES + 1],
    };
    let mut r = Recent::waiting();
    assert_eq!(r.keep(big(1)), Kept::TooLarge);
    r.attach(Box::new(Fake::default()));
    assert_eq!(r.note(), Some(TOO_LARGE));
    assert!(r.list(0.0).is_empty());
    // A model kept while waiting replays on attach and clears an older note.
    let mut r = Recent::waiting();
    r.keep(big(1));
    r.keep(record(2, 2.0, 10));
    r.attach(Box::new(Fake::default()));
    assert_eq!(r.note(), None);
    assert_eq!(ids(&r), [id(2)]);
}

#[test]
fn the_not_saved_note_goes_when_a_kept_model_opens() {
    let (mut r, fake) = recent();
    r.keep(record(1, 1.0, 10));
    fake.full.set(true);
    r.keep(record(2, 2.0, 10));
    assert_eq!(r.note(), Some(NOT_SAVED));
    r.open(&id(1), 3.0, Box::new(|found| assert!(found.is_some())));
    assert_eq!(r.note(), None);
}

#[test]
fn a_model_is_kept_only_when_the_person_asks() {
    let (mut r, fake) = recent();
    assert!(!r.can_keep());
    assert_eq!(r.keep_offered(), None);
    r.offer(record(1, 1.0, 10));
    assert!(r.can_keep());
    assert!(fake.records.borrow().is_empty(), "offering stores nothing");
    assert!(r.list(0.0).is_empty());
    assert_eq!(r.keep_offered(), Some(Kept::Yes));
    assert!(!r.can_keep(), "kept: nothing more to offer");
    assert_eq!(ids(&r), [id(1)]);
    assert_eq!(r.keep_offered(), None);
}

#[test]
fn a_new_offer_replaces_the_old_and_a_too_large_one_is_not_offered() {
    let (mut r, _) = recent();
    r.offer(record(1, 1.0, 10));
    r.offer(record(2, 2.0, 10));
    r.keep_offered();
    assert_eq!(ids(&r), [id(2)]);
    r.offer(Record {
        info: Info {
            id: id(3),
            name: "Big".into(),
            time: 3.0,
            size: MAX_KEPT_BYTES + 1,
        },
        key: [3; 32],
        blob: vec![0; MAX_KEPT_BYTES + 1],
    });
    assert!(!r.can_keep());
    assert_eq!(r.note(), Some(TOO_LARGE));
}

#[test]
fn an_offer_made_before_the_storage_opens_is_kept_when_asked() {
    let mut r = Recent::waiting();
    r.offer(record(1, 1.0, 10));
    assert_eq!(r.keep_offered(), Some(Kept::Later));
    let fake = Fake::default();
    r.attach(Box::new(fake));
    assert_eq!(ids(&r), [id(1)]);
}

#[test]
fn opening_a_kept_model_ends_the_offer_and_a_missing_one_changes_nothing() {
    let (mut r, _) = recent();
    r.offer(record(1, 1.0, 10));
    r.keep_offered();
    r.offer(record(2, 2.0, 10));
    r.offer(Record {
        info: Info {
            id: id(9),
            name: "Big".into(),
            time: 9.0,
            size: MAX_KEPT_BYTES + 1,
        },
        key: [9; 32],
        blob: vec![0; MAX_KEPT_BYTES + 1],
    });
    assert_eq!(r.note(), Some(TOO_LARGE));
    // Not kept any more: opening it opens nothing, so the note stays.
    r.open(&id(7), 5.0, Box::new(|found| assert!(found.is_none())));
    assert_eq!(r.note(), Some(TOO_LARGE));
    r.open(&id(1), 5.0, Box::new(|found| assert!(found.is_some())));
    assert_eq!(r.note(), None);
    assert!(!r.can_keep());
}
