//! The browser's IndexedDB as the Recent list's `Storage`: database `tenon-viewer`,
//! object store `recent` (`{ id, key, name, time, blob }`, keyed by `id`) beside `recent-info`
//! (`{ id, name, time, size }`), which lets the list load without reading every blob (up to
//! five of 64 MiB). The list is read from memory (`infos` is loaded once when the database
//! opens and kept in step with every write); writes finish in the background and a failed one
//! (storage full) takes its entry back out of the list. `time` in `recent` is when the model
//! was first kept; the list goes by `recent-info`, which opening a model updates.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Event, IdbDatabase, IdbObjectStoreParameters, IdbOpenDbRequest, IdbRequest, IdbTransaction,
    IdbTransactionMode, IdbVersionChangeEvent,
};

use crate::files::{describe, window};
use crate::recent::{DB_NAME, INFO_STORE, Info, Record, STORE, Storage};

/// The keys of the kept models are never logged.
fn log(text: &str) {
    web_sys::console::error_1(&format!("Tenon viewer storage: {text}").into());
}

type Infos = Rc<RefCell<Vec<Info>>>;

/// IndexedDB, opened.
pub struct Idb {
    db: IdbDatabase,
    infos: Infos,
    /// Whether the last write failed.
    failed: Rc<Cell<bool>>,
}

/// A storage that keeps nothing: the browser would not open IndexedDB.
pub struct Unavailable;

impl Storage for Unavailable {
    fn available(&self) -> bool {
        false
    }

    fn infos(&self) -> Vec<Info> {
        Vec::new()
    }

    fn put(&mut self, _: Record, _: Vec<String>) {}

    fn touch(&mut self, _: &str, _: f64) {}

    fn remove(&mut self, _: &str) {}

    fn get(&self, _: &str, done: Box<dyn FnOnce(Option<Record>)>) {
        done(None);
    }
}

/// Resolves with the request's result, or rejects when it fails.
fn wait(request: &IdbRequest) -> Promise {
    let request = request.clone();
    Promise::new(&mut move |resolve, reject| {
        let result = request.clone();
        let ok = Closure::once_into_js(move |_: Event| {
            let _ = resolve.call1(
                &JsValue::NULL,
                &result.result().unwrap_or(JsValue::UNDEFINED),
            );
        });
        let failed = Closure::once_into_js(move |_: Event| {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("the request failed"));
        });
        request.set_onsuccess(Some(ok.unchecked_ref()));
        request.set_onerror(Some(failed.unchecked_ref()));
    })
}

/// [`wait`] for an open request, which is also refused when another tab holds an older
/// version of the database open (`blocked`) rather than waiting for it forever.
fn wait_open(request: &IdbOpenDbRequest) -> Promise {
    let promise = wait(request);
    let blocked = Promise::new(&mut |_, reject| {
        let refuse = Closure::once_into_js(move |_: Event| {
            let _ = reject.call1(
                &JsValue::NULL,
                &JsValue::from_str("the database is blocked"),
            );
        });
        request.set_onblocked(Some(refuse.unchecked_ref()));
    });
    Promise::race(&Array::of2(&promise, &blocked))
}

/// Opens the database, giving up (None) after `ms` milliseconds: a browser that never answers
/// must not hold the viewer back.
pub async fn open_within(ms: i32) -> Option<Idb> {
    let slot: Rc<RefCell<Option<Option<Idb>>>> = Rc::default();
    let opening = {
        let slot = slot.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            *slot.borrow_mut() = Some(open().await);
            Ok(JsValue::UNDEFINED)
        })
    };
    let timer = Promise::new(&mut |resolve, _| {
        let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(Promise::race(&Array::of2(&opening, &timer))).await;
    slot.borrow_mut().take().flatten()
}

/// Resolves when the transaction has completed (its writes are kept), rejects when it aborts.
fn complete(transaction: &IdbTransaction) -> Promise {
    let transaction = transaction.clone();
    Promise::new(&mut move |resolve, reject| {
        let reject_abort = reject.clone();
        let ok = Closure::once_into_js(move |_: Event| {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let failed = Closure::once_into_js(move |_: Event| {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("the write failed"));
        });
        let aborted = Closure::once_into_js(move |_: Event| {
            let _ = reject_abort.call1(&JsValue::NULL, &JsValue::from_str("the write was refused"));
        });
        transaction.set_oncomplete(Some(ok.unchecked_ref()));
        transaction.set_onerror(Some(failed.unchecked_ref()));
        transaction.set_onabort(Some(aborted.unchecked_ref()));
    })
}

fn set(object: &Object, name: &str, value: &JsValue) {
    let _ = Reflect::set(object, &name.into(), value);
}

fn field(object: &JsValue, name: &str) -> JsValue {
    Reflect::get(object, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

fn info_object(info: &Info) -> Object {
    let o = Object::new();
    set(&o, "id", &info.id.as_str().into());
    set(&o, "name", &info.name.as_str().into());
    set(&o, "time", &info.time.into());
    set(&o, "size", &(info.size as f64).into());
    o
}

fn info_of(value: &JsValue) -> Option<Info> {
    Some(Info {
        id: field(value, "id").as_string()?,
        name: field(value, "name").as_string()?,
        time: field(value, "time").as_f64()?,
        size: field(value, "size").as_f64()? as usize,
    })
}

fn record_object(record: &Record) -> Object {
    let o = Object::new();
    set(&o, "id", &record.info.id.as_str().into());
    set(&o, "key", &Uint8Array::from(&record.key[..]));
    set(&o, "name", &record.info.name.as_str().into());
    set(&o, "time", &record.info.time.into());
    set(&o, "blob", &Uint8Array::from(&record.blob[..]));
    o
}

fn record_of(value: &JsValue, info: Info) -> Option<Record> {
    let key: [u8; 32] = field(value, "key")
        .dyn_into::<Uint8Array>()
        .ok()?
        .to_vec()
        .try_into()
        .ok()?;
    let blob = field(value, "blob").dyn_into::<Uint8Array>().ok()?.to_vec();
    Some(Record { info, key, blob })
}

/// Opens (creating on first use) the viewer's database and reads the list. None when the
/// browser has no IndexedDB or will not open it.
pub async fn open() -> Option<Idb> {
    let factory = window().indexed_db().ok()??;
    let request: IdbOpenDbRequest = factory.open_with_u32(DB_NAME, 1).ok()?;
    let upgrade = Closure::once_into_js({
        let request = request.clone();
        move |event: Event| {
            // Version 1 creates the stores; a later version must add to them, not redo this.
            let old = event
                .dyn_ref::<IdbVersionChangeEvent>()
                .map_or(0.0, IdbVersionChangeEvent::old_version);
            if old >= 1.0 {
                return;
            }
            let Ok(db) = request.result().and_then(|r| r.dyn_into::<IdbDatabase>()) else {
                return;
            };
            let params = IdbObjectStoreParameters::new();
            params.set_key_path(&"id".into());
            for store in [STORE, INFO_STORE] {
                let _ = db.create_object_store_with_optional_parameters(store, &params);
            }
        }
    });
    request.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    let db: IdbDatabase = JsFuture::from(wait_open(&request))
        .await
        .map_err(|e| log(&describe(&e)))
        .ok()?
        .dyn_into()
        .ok()?;
    let infos = read_infos(&db).await.unwrap_or_default();
    Some(Idb {
        db,
        infos: Rc::new(RefCell::new(infos)),
        failed: Rc::default(),
    })
}

async fn read_infos(db: &IdbDatabase) -> Result<Vec<Info>, JsValue> {
    let store = db
        .transaction_with_str(INFO_STORE)?
        .object_store(INFO_STORE)?;
    let all = JsFuture::from(wait(&store.get_all()?)).await?;
    Ok(Array::from(&all)
        .iter()
        .filter_map(|v| info_of(&v))
        .collect())
}

async fn write(db: &IdbDatabase, record: &Record) -> Result<(), JsValue> {
    let stores = Array::of2(&STORE.into(), &INFO_STORE.into());
    let tx = db.transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)?;
    tx.object_store(STORE)?.put(&record_object(record))?;
    tx.object_store(INFO_STORE)?
        .put(&info_object(&record.info))?;
    JsFuture::from(complete(&tx)).await.map(|_| ())
}

async fn write_info(db: &IdbDatabase, info: &Info) -> Result<(), JsValue> {
    let tx = db.transaction_with_str_and_mode(INFO_STORE, IdbTransactionMode::Readwrite)?;
    tx.object_store(INFO_STORE)?.put(&info_object(info))?;
    JsFuture::from(complete(&tx)).await.map(|_| ())
}

async fn delete(db: &IdbDatabase, id: &str) -> Result<(), JsValue> {
    let stores = Array::of2(&STORE.into(), &INFO_STORE.into());
    let tx = db.transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)?;
    tx.object_store(STORE)?.delete(&id.into())?;
    tx.object_store(INFO_STORE)?.delete(&id.into())?;
    JsFuture::from(complete(&tx)).await.map(|_| ())
}

async fn read(db: &IdbDatabase, info: Info) -> Result<Option<Record>, JsValue> {
    let store = db.transaction_with_str(STORE)?.object_store(STORE)?;
    let found = JsFuture::from(wait(&store.get(&info.id.as_str().into())?)).await?;
    Ok((!found.is_undefined())
        .then(|| record_of(&found, info))
        .flatten())
}

impl Storage for Idb {
    fn infos(&self) -> Vec<Info> {
        self.infos.borrow().clone()
    }

    fn put(&mut self, record: Record, then_remove: Vec<String>) {
        {
            let mut infos = self.infos.borrow_mut();
            infos.retain(|i| i.id != record.info.id);
            infos.push(record.info.clone());
        }
        let (db, infos, failed) = (self.db.clone(), self.infos.clone(), self.failed.clone());
        spawn_local(async move {
            match write(&db, &record).await {
                Ok(()) => {
                    failed.set(false);
                    // Now that the new one is safe, the oldest may go.
                    for id in then_remove {
                        infos.borrow_mut().retain(|i| i.id != id);
                        if let Err(e) = delete(&db, &id).await {
                            log(&describe(&e));
                        }
                    }
                }
                Err(e) => {
                    log(&describe(&e));
                    // Not kept (storage full, or refused): the list does not offer it, and
                    // nothing older was dropped for it.
                    failed.set(true);
                    infos.borrow_mut().retain(|i| i.id != record.info.id);
                }
            }
        });
    }

    fn write_failed(&self) -> bool {
        self.failed.get()
    }

    fn clear_failure(&mut self) {
        self.failed.set(false);
    }

    fn touch(&mut self, id: &str, time: f64) {
        let updated = {
            let mut infos = self.infos.borrow_mut();
            infos.iter_mut().find(|i| i.id == id).map(|i| {
                i.time = time;
                i.clone()
            })
        };
        if let Some(info) = updated {
            let db = self.db.clone();
            spawn_local(async move {
                if let Err(e) = write_info(&db, &info).await {
                    log(&describe(&e));
                }
            });
        }
    }

    fn remove(&mut self, id: &str) {
        self.infos.borrow_mut().retain(|i| i.id != id);
        let (db, id) = (self.db.clone(), id.to_string());
        spawn_local(async move {
            if let Err(e) = delete(&db, &id).await {
                log(&describe(&e));
            }
        });
    }

    fn get(&self, id: &str, done: Box<dyn FnOnce(Option<Record>)>) {
        let Some(info) = self.infos.borrow().iter().find(|i| i.id == id).cloned() else {
            done(None);
            return;
        };
        let db = self.db.clone();
        spawn_local(async move {
            match read(&db, info).await {
                Ok(record) => done(record),
                Err(e) => {
                    log(&describe(&e));
                    done(None);
                }
            }
        });
    }
}
