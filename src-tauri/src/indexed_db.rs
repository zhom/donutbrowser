//! IndexedDB in storage-state files, in the shape Playwright reads and writes
//! (`origins[].indexedDB`).
//!
//! IndexedDB on disk is a LevelDB of V8-serialized values that only the
//! browser can read back faithfully, so this module asks the browser: it opens
//! the closed profile headless for a moment, loads a blank page of each origin
//! that the browser itself answers (`Fetch.fulfillRequest`, so no request
//! leaves the computer and no site code runs), and reads or writes the
//! databases there with the page's own IndexedDB API.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use crate::profile::BrowserProfile;
use crate::wayfern_cdp::WayfernSession;

/// The path of the blank page the browser answers itself.
const BLANK_PATH: &str = "/__donut_storage__";
/// The most a whole export may hold, so the file it lands in can be imported.
const MAX_EXPORT_CHARS: usize = 24 * 1024 * 1024;
const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(120);

fn code(code: &str) -> String {
  crate::backend_error(code)
}

/// The origins with IndexedDB data in a profile directory: each origin's
/// folder is named `<scheme>_<host>_<port>`, with `.indexeddb.leveldb` after it
/// on older browsers, port 0 being the scheme's default.
pub fn origins_on_disk(profile_data: &Path) -> Vec<String> {
  let Ok(entries) = std::fs::read_dir(profile_data.join("Default").join("IndexedDB")) else {
    return Vec::new();
  };
  let mut origins: Vec<String> = entries
    .flatten()
    .filter_map(|entry| origin_of_folder(&entry.file_name().to_string_lossy()))
    .collect();
  origins.sort();
  origins.dedup();
  origins
}

fn origin_of_folder(name: &str) -> Option<String> {
  // Older browsers name the folder `<origin>.indexeddb.leveldb`; 152 names it
  // after the origin alone and keeps one folder per database inside.
  if name.contains(".indexeddb.") && !name.ends_with(".indexeddb.leveldb") {
    return None;
  }
  let stem = name.strip_suffix(".indexeddb.leveldb").unwrap_or(name);
  let (scheme, rest) = stem.split_once('_')?;
  if scheme != "https" && scheme != "http" {
    return None;
  }
  let (host, port) = rest.rsplit_once('_')?;
  let port: u16 = port.parse().ok()?;
  if host.is_empty()
    || !host
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
  {
    return None;
  }
  Some(if port == 0 {
    format!("{scheme}://{host}")
  } else {
    format!("{scheme}://{host}:{port}")
  })
}

/// The serializer both scripts share, compatible with the encoded values in
/// Playwright's storage state (`keyEncoded`, `valueEncoded`).
const CODEC_JS: &str = r#"
const KINDS = { Int8Array: 'i8', Uint8Array: 'ui8', Uint8ClampedArray: 'ui8c', Int16Array: 'i16',
  Uint16Array: 'ui16', Int32Array: 'i32', Uint32Array: 'ui32', Float32Array: 'f32',
  Float64Array: 'f64', BigInt64Array: 'bi64', BigUint64Array: 'bui64' };
const CTORS = Object.fromEntries(Object.entries(KINDS).map(([name, kind]) => [kind, globalThis[name]]));
class Unsupported extends Error {}
const plainProto = (v) => { const p = Object.getPrototypeOf(v); return p === Object.prototype || p === null; };
const isPlain = (v, seen = new Set()) => {
  if (v === null || typeof v === 'string' || typeof v === 'boolean') return true;
  if (typeof v === 'number') return Number.isFinite(v) && !Object.is(v, -0);
  if (typeof v !== 'object' || seen.has(v)) return false;
  seen.add(v);
  if (Array.isArray(v)) return v.every((item) => isPlain(item, seen));
  if (!plainProto(v)) return false;
  return Object.keys(v).every((key) => v[key] !== undefined && isPlain(v[key], seen));
};
const toBase64 = (view) => {
  const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
  let text = '';
  for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(text);
};
const encode = (value) => {
  const ids = new Map();
  let last = 0;
  const walk = (v) => {
    if (v === undefined) return { v: 'undefined' };
    if (v === null) return { v: 'null' };
    if (typeof v === 'number') {
      if (Number.isNaN(v)) return { v: 'NaN' };
      if (v === Infinity) return { v: 'Infinity' };
      if (v === -Infinity) return { v: '-Infinity' };
      if (Object.is(v, -0)) return { v: '-0' };
      return v;
    }
    if (typeof v === 'boolean' || typeof v === 'string') return v;
    if (typeof v === 'bigint') return { bi: v.toString() };
    if (typeof v !== 'object') throw new Unsupported();
    if (v instanceof Date) return { d: v.toJSON() };
    if (v instanceof URL) return { u: v.toJSON() };
    if (v instanceof RegExp) return { r: { p: v.source, f: v.flags } };
    if (v instanceof Error) return { e: { n: v.name, m: v.message, s: v.stack } };
    if (ArrayBuffer.isView(v) && KINDS[v.constructor.name]) {
      return { ta: { b: toBase64(v), k: KINDS[v.constructor.name] } };
    }
    if (ids.has(v)) return { ref: ids.get(v) };
    if (!Array.isArray(v) && !plainProto(v)) throw new Unsupported();
    const id = ++last;
    ids.set(v, id);
    if (Array.isArray(v)) return { a: v.map(walk), id };
    return { o: Object.keys(v).map((k) => ({ k, v: walk(v[k]) })), id };
  };
  return walk(value);
};
const decode = (value) => {
  const refs = new Map();
  const walk = (x) => {
    if (x === null || typeof x !== 'object') return x;
    if ('v' in x) return { undefined: undefined, null: null, NaN: NaN, Infinity: Infinity,
      '-Infinity': -Infinity, '-0': -0 }[x.v];
    if ('bi' in x) return BigInt(x.bi);
    if ('d' in x) return new Date(x.d);
    if ('u' in x) return new URL(x.u);
    if ('r' in x) return new RegExp(x.r.p, x.r.f);
    if ('e' in x) { const e = new Error(x.e.m); e.name = x.e.n; e.stack = x.e.s; return e; }
    if ('ta' in x) {
      const text = atob(x.ta.b);
      const bytes = new Uint8Array(text.length);
      for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i);
      const Ctor = CTORS[x.ta.k];
      if (!Ctor) throw new Error('unknown typed array ' + x.ta.k);
      return new Ctor(bytes.buffer);
    }
    if ('ref' in x) return refs.get(x.ref);
    if ('a' in x) { const out = []; refs.set(x.id, out); for (const item of x.a) out.push(walk(item)); return out; }
    if ('o' in x) { const out = {}; refs.set(x.id, out); for (const { k, v } of x.o) out[k] = walk(v); return out; }
    throw new Error('unknown encoded value');
  };
  return walk(value);
};
const request = (req) => new Promise((resolve, reject) => {
  req.onsuccess = () => resolve(req.result);
  req.onerror = () => reject(req.error);
});
const keyPathOf = (keyPath) => typeof keyPath === 'string' ? { keyPath }
  : Array.isArray(keyPath) ? { keyPathArray: [...keyPath] } : {};
"#;

/// Every database of the page's origin, Playwright's shape. Records the
/// format cannot carry (a Blob, a Map) are counted in `skipped`, not lost
/// silently.
const EXPORT_JS: &str = r#"
(async () => {
  const LIMIT = __LIMIT__;
  let size = 0;
  let skipped = 0;
  const databases = [];
  for (const info of await indexedDB.databases()) {
    if (!info.name) continue;
    const db = await request(indexedDB.open(info.name));
    const stores = [];
    for (const name of Array.from(db.objectStoreNames)) {
      const store = db.transaction(name, 'readonly').objectStore(name);
      const records = [];
      await new Promise((resolve, reject) => {
        const cursor = store.openCursor();
        cursor.onerror = () => reject(cursor.error);
        cursor.onsuccess = () => {
          const at = cursor.result;
          if (!at) return resolve();
          try {
            const record = {};
            if (store.keyPath === null) {
              if (isPlain(at.primaryKey)) record.key = at.primaryKey;
              else record.keyEncoded = encode(at.primaryKey);
            }
            if (isPlain(at.value)) record.value = at.value;
            else record.valueEncoded = encode(at.value);
            size += JSON.stringify(record).length;
            if (size > LIMIT) return reject(new Error('DONUT_TOO_LARGE'));
            records.push(record);
          } catch (error) {
            if (!(error instanceof Unsupported)) return reject(error);
            skipped += 1;
          }
          at.continue();
        };
      });
      stores.push({
        name,
        autoIncrement: store.autoIncrement,
        ...keyPathOf(store.keyPath),
        records,
        indexes: Array.from(store.indexNames).map((indexName) => {
          const index = store.index(indexName);
          return { name: indexName, ...keyPathOf(index.keyPath), multiEntry: index.multiEntry, unique: index.unique };
        }),
      });
    }
    db.close();
    databases.push({ name: info.name, version: info.version, stores });
  }
  return { databases, skipped };
})()
"#;

/// Replace the databases the file names with its copies; other databases of
/// the origin stay as they are.
const IMPORT_JS: &str = r#"
(async () => {
  const DATABASES = __DATA__;
  let records = 0;
  for (const source of DATABASES) {
    await new Promise((resolve, reject) => {
      const req = indexedDB.deleteDatabase(source.name);
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error);
    });
    const open = indexedDB.open(source.name, source.version);
    open.onupgradeneeded = () => {
      const db = open.result;
      for (const store of source.stores) {
        const options = { autoIncrement: !!store.autoIncrement };
        const keyPath = store.keyPathArray ?? store.keyPath;
        if (keyPath !== undefined && keyPath !== null) options.keyPath = keyPath;
        const created = db.createObjectStore(store.name, options);
        for (const index of store.indexes ?? []) {
          created.createIndex(index.name, index.keyPathArray ?? index.keyPath,
            { multiEntry: !!index.multiEntry, unique: !!index.unique });
        }
      }
    };
    const db = await request(open);
    for (const store of source.stores) {
      if (!store.records?.length) continue;
      await new Promise((resolve, reject) => {
        const tx = db.transaction(store.name, 'readwrite');
        const target = tx.objectStore(store.name);
        for (const record of store.records) {
          const value = 'valueEncoded' in record ? decode(record.valueEncoded) : record.value;
          if (target.keyPath === null) {
            target.put(value, 'keyEncoded' in record ? decode(record.keyEncoded) : record.key);
          } else {
            target.put(value);
          }
          records += 1;
        }
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
        tx.onabort = () => reject(tx.error);
      });
    }
    db.close();
  }
  return { records };
})()
"#;

/// The databases an import file names for one origin, checked for the shape
/// the import script needs before anything is deleted.
pub fn checked_databases(databases: &[Value]) -> Result<(), String> {
  let valid = databases.iter().all(|db| {
    db.get("name")
      .and_then(Value::as_str)
      .is_some_and(|n| !n.is_empty())
      && db
        .get("version")
        .and_then(Value::as_u64)
        .is_some_and(|v| v >= 1)
      && db
        .get("stores")
        .and_then(Value::as_array)
        .is_some_and(|stores| {
          stores.iter().all(|store| {
            store.get("name").and_then(Value::as_str).is_some()
              && store
                .get("records")
                .is_none_or(|records| records.is_array())
              && store
                .get("indexes")
                .is_none_or(|indexes| indexes.is_array())
          })
        })
  });
  if valid {
    Ok(())
  } else {
    Err(code("STORAGE_STATE_INVALID"))
  }
}

/// A headless browser on `profile`, closed again when the work is done
/// whatever it returned.
pub async fn with_headless_browser<T, F>(
  app_handle: &tauri::AppHandle,
  profile: &BrowserProfile,
  work: F,
) -> Result<T, String>
where
  F: for<'a> FnOnce(
    &'a mut WayfernSession,
  ) -> futures_util::future::BoxFuture<'a, Result<T, String>>,
{
  let port = {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
    listener
      .local_addr()
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?
      .port()
  };
  let started = std::time::Instant::now();
  let running = crate::browser_runner::launch_browser_profile_impl(
    app_handle.clone(),
    profile.clone(),
    Some("about:blank".to_string()),
    crate::browser_runner::LaunchOptions::automation(Some(port), true),
  )
  .await?;
  let outcome = async {
    let target = crate::cdp_target::resolve(&running)
      .await
      .map_err(|e| crate::backend_error_with_detail("INDEXED_DB_UNAVAILABLE", e))?;
    let mut session = WayfernSession::open(&target)
      .await
      .map_err(|e| crate::backend_error_with_detail("INDEXED_DB_UNAVAILABLE", e))?;
    let result = prepare(&mut session).await;
    let result = match result {
      Ok(()) => work(&mut session).await,
      Err(e) => Err(e),
    };
    session.close().await;
    result
  }
  .await;
  if let Err(e) = crate::browser_runner::kill_browser_profile(app_handle.clone(), running).await {
    log::warn!(
      "Headless browser not closed after IndexedDB work profile={} err=\"{e}\"",
      profile.id
    );
  }
  log::info!(
    "IndexedDB browser closed profile={} ok={} elapsed_ms={}",
    profile.id,
    outcome.is_ok(),
    started.elapsed().as_millis()
  );
  outcome
}

async fn prepare(session: &mut WayfernSession) -> Result<(), String> {
  let unavailable = |e: crate::wayfern_cdp::WayfernError| {
    crate::backend_error_with_detail("INDEXED_DB_UNAVAILABLE", e)
  };
  session
    .call("Page.enable", json!({}))
    .await
    .map_err(unavailable)?;
  // A service worker of the origin would answer the navigation itself, with
  // the site's own page and scripts.
  session
    .call("Network.enable", json!({}))
    .await
    .map_err(unavailable)?;
  session
    .call("Network.setBypassServiceWorker", json!({ "bypass": true }))
    .await
    .map_err(unavailable)?;
  // Every request of the page pauses here. Only the blank page is answered;
  // anything else it asks for (a favicon) waits until the browser closes, so
  // nothing reaches the network.
  session
    .call(
      "Fetch.enable",
      json!({ "patterns": [{ "urlPattern": "*", "requestStage": "Request" }] }),
    )
    .await
    .map_err(unavailable)?;
  Ok(())
}

/// Load the blank page of `origin` that the browser answers itself.
async fn open_origin(session: &mut WayfernSession, origin: &str) -> Result<(), String> {
  let unavailable = |e: crate::wayfern_cdp::WayfernError| {
    crate::backend_error_with_detail("INDEXED_DB_UNAVAILABLE", e)
  };
  let url = format!("{origin}{BLANK_PATH}");
  session
    .send("Page.navigate", json!({ "url": url }))
    .await
    .map_err(unavailable)?;
  let request_id = loop {
    let Some((_, paused)) = session
      .await_any_event(&["Fetch.requestPaused"], STEP_TIMEOUT)
      .await
      .map_err(unavailable)?
    else {
      return Err(code("INDEXED_DB_UNAVAILABLE"));
    };
    if paused.pointer("/request/url").and_then(Value::as_str) == Some(url.as_str()) {
      break paused
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    }
  };
  use base64::Engine as _;
  let body = base64::engine::general_purpose::STANDARD.encode("<!doctype html><title></title>");
  session
    .call(
      "Fetch.fulfillRequest",
      json!({
        "requestId": request_id,
        "responseCode": 200,
        "responseHeaders": [{ "name": "Content-Type", "value": "text/html; charset=utf-8" }],
        "body": body,
      }),
    )
    .await
    .map_err(unavailable)?;
  // The load event may already have passed while the reply above was read, so
  // the page is asked where it is rather than waited on.
  let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
  loop {
    let reply = session
      .call(
        "Runtime.evaluate",
        json!({
          "expression": "location.origin + '|' + document.readyState",
          "returnByValue": true,
        }),
      )
      .await;
    let at = reply.ok().and_then(|r| {
      r.pointer("/result/value")
        .and_then(Value::as_str)
        .map(str::to_string)
    });
    if at.as_deref() == Some(&format!("{origin}|complete")) {
      return Ok(());
    }
    if tokio::time::Instant::now() >= deadline {
      return Err(code("INDEXED_DB_UNAVAILABLE"));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
}

async fn run_script(session: &mut WayfernSession, script: String) -> Result<Value, String> {
  let reply = session
    .call_with_timeout(
      "Runtime.evaluate",
      json!({ "expression": script, "awaitPromise": true, "returnByValue": true }),
      SCRIPT_TIMEOUT,
    )
    .await
    .map_err(|e| crate::backend_error_with_detail("INDEXED_DB_UNAVAILABLE", e))?;
  if let Some(exception) = reply.get("exceptionDetails") {
    let text = exception
      .pointer("/exception/description")
      .and_then(Value::as_str)
      .unwrap_or_default();
    if text.contains("DONUT_TOO_LARGE") {
      return Err(code("INDEXED_DB_TOO_LARGE"));
    }
    return Err(crate::backend_error_with_detail(
      "INDEXED_DB_UNAVAILABLE",
      text.lines().next().unwrap_or("script failed"),
    ));
  }
  Ok(
    reply
      .pointer("/result/value")
      .cloned()
      .unwrap_or(Value::Null),
  )
}

/// What an export found: each origin's databases, and the records skipped.
pub struct Exported {
  pub origins: Vec<(String, Vec<Value>)>,
  pub skipped: u64,
}

pub async fn export(session: &mut WayfernSession, origins: &[String]) -> Result<Exported, String> {
  let mut exported = Exported {
    origins: Vec::new(),
    skipped: 0,
  };
  let mut budget = MAX_EXPORT_CHARS;
  for origin in origins {
    open_origin(session, origin).await?;
    let script = format!(
      "{CODEC_JS}\n{}",
      EXPORT_JS.replace("__LIMIT__", &budget.to_string())
    );
    let found = run_script(session, script).await?;
    exported.skipped += found.get("skipped").and_then(Value::as_u64).unwrap_or(0);
    let databases = found
      .get("databases")
      .and_then(Value::as_array)
      .cloned()
      .unwrap_or_default();
    budget = budget.saturating_sub(
      serde_json::to_string(&databases)
        .map(|s| s.len())
        .unwrap_or(0),
    );
    if !databases.is_empty() {
      exported.origins.push((origin.clone(), databases));
    }
  }
  Ok(exported)
}

/// Write each origin's databases; answers how many records went in.
pub async fn import(
  session: &mut WayfernSession,
  origins: &[(String, Vec<Value>)],
) -> Result<u64, String> {
  let mut records = 0;
  for (origin, databases) in origins {
    open_origin(session, origin).await?;
    let data = serde_json::to_string(databases).map_err(|e| e.to_string())?;
    let script = format!("{CODEC_JS}\n{}", IMPORT_JS.replace("__DATA__", &data));
    let written = run_script(session, script).await?;
    records += written.get("records").and_then(Value::as_u64).unwrap_or(0);
  }
  Ok(records)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_database_folder_names_its_origin() {
    assert_eq!(
      origin_of_folder("https_example.com_0.indexeddb.leveldb").as_deref(),
      Some("https://example.com")
    );
    assert_eq!(
      origin_of_folder("http_127.0.0.1_8080.indexeddb.leveldb").as_deref(),
      Some("http://127.0.0.1:8080")
    );
    assert_eq!(
      origin_of_folder("http_127.0.0.1_64554").as_deref(),
      Some("http://127.0.0.1:64554")
    );
    assert_eq!(origin_of_folder("file__0.indexeddb.leveldb"), None);
    assert_eq!(origin_of_folder("https_example.com_0.indexeddb.blob"), None);
    assert_eq!(origin_of_folder("https_a b_0.indexeddb.leveldb"), None);
  }

  #[test]
  fn only_a_well_formed_database_list_is_imported() {
    let good = json!([{ "name": "app", "version": 3, "stores": [
      { "name": "kv", "autoIncrement": false, "records": [{ "key": "a", "value": 1 }], "indexes": [] }
    ]}]);
    assert!(checked_databases(good.as_array().unwrap()).is_ok());
    for bad in [
      json!([{ "version": 1, "stores": [] }]),
      json!([{ "name": "app", "version": 0, "stores": [] }]),
      json!([{ "name": "app", "version": 1, "stores": [{ "name": "kv", "records": {} }] }]),
    ] {
      assert!(checked_databases(bad.as_array().unwrap()).is_err(), "{bad}");
    }
  }

  #[test]
  fn the_scripts_take_their_data_where_they_expect_it() {
    assert!(EXPORT_JS.contains("__LIMIT__"));
    assert!(IMPORT_JS.contains("__DATA__"));
    assert!(CODEC_JS.contains("const encode") && CODEC_JS.contains("const decode"));
  }
}
