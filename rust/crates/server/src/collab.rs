//! Yjs collaborative-editing layer — a port of the y-websocket binary sync
//! protocol and the per-schedule CRDT doc handling from `server.py`.
//!
//! Wire compatibility: the message framing (lib0 varint + length-prefixed
//! buffers) is reproduced byte-for-byte, and the payloads are Yjs **v1**
//! state-vector / update encodings produced by the `yrs` crate. pycrdt (the
//! Python side) and `yrs` both implement the same Yjs v1 binary spec, so a
//! pycrdt client interoperates with this server (a captured-pycrdt-update
//! fixture test pins that cross-implementation compatibility).
//!
//! Doc model (identical to Python): a top-level Map named `"schedule"`, keyed
//! `"{day}:{boat}"`, each value a Y.Array of member-name strings.
//!
//! Ownership/async note (flagged for review): the per-schedule `Doc` and its
//! broadcast `Sender` live in a `Mutex<HashMap>`. A `yrs` transaction borrows
//! the doc and must **never** be held across an `.await` (it would deadlock the
//! doc's internal lock), so every transaction here is confined to a synchronous
//! block; only owned byte buffers cross `.await` points. Each connection clones
//! the `Doc` handle (cheap — it shares the underlying store via `Arc`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use boatsched_core::ScheduleRow;
use tokio::sync::broadcast;
use yrs::types::ToJson;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Any, Array, ArrayPrelim, ArrayRef, Doc, Map, ReadTxn, StateVector, Transact, Update};

// y-protocol message/sub types (mirror the module-level constants in server.py).
pub const MSG_SYNC: u64 = 0;
pub const MSG_AWARENESS: u64 = 1;
pub const SYNC_STEP1: u64 = 0;
pub const SYNC_STEP2: u64 = 1;
pub const SYNC_UPDATE: u64 = 2;

// ---------------------------------------------------------------------------
// lib0 varint + length-prefixed buffer framing (byte-identical to server.py)
// ---------------------------------------------------------------------------

pub fn write_varint(mut n: u64, out: &mut Vec<u8>) {
    while n > 127 {
        out.push(((n & 0x7F) | 0x80) as u8);
        n >>= 7;
    }
    out.push((n & 0x7F) as u8);
}

/// Read a varint at `pos`, returning `(value, new_pos)` or `None` past the end.
pub fn read_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let b = *buf.get(pos)?;
        pos += 1;
        result |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Some((result, pos))
}

fn write_buf(data: &[u8], out: &mut Vec<u8>) {
    write_varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

fn read_buf(buf: &[u8], pos: usize) -> Option<(&[u8], usize)> {
    let (n, pos) = read_varint(buf, pos)?;
    let end = pos.checked_add(n as usize)?;
    let slice = buf.get(pos..end)?;
    Some((slice, end))
}

pub fn encode_sync_step1(sv: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(MSG_SYNC, &mut out);
    write_varint(SYNC_STEP1, &mut out);
    write_buf(sv, &mut out);
    out
}

pub fn encode_sync_step2(update: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(MSG_SYNC, &mut out);
    write_varint(SYNC_STEP2, &mut out);
    write_buf(update, &mut out);
    out
}

pub fn encode_update_msg(update: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(MSG_SYNC, &mut out);
    write_varint(SYNC_UPDATE, &mut out);
    write_buf(update, &mut out);
    out
}

/// `(msg_type, sub_type, payload)` or `None` on a malformed frame. Mirrors
/// `_parse_message`.
pub fn parse_message(buf: &[u8]) -> Option<(u64, u64, Vec<u8>)> {
    let (msg_type, pos) = read_varint(buf, 0)?;
    let (sub_type, pos) = read_varint(buf, pos)?;
    let (payload, _) = read_buf(buf, pos)?;
    Some((msg_type, sub_type, payload.to_vec()))
}

// ---------------------------------------------------------------------------
// Doc helpers
// ---------------------------------------------------------------------------

/// Build a fresh doc from solver result rows and return its full v1 update —
/// the bytes stored as `schedule.yjs_state`. Mirrors `_init_doc_from_result`
/// (groups by `"{day}:{boat}"`, inserts in sorted key order).
pub fn init_doc_update_from_rows(rows: &[ScheduleRow]) -> Vec<u8> {
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for r in rows {
        groups
            .entry(format!("{}:{}", r.day, r.boat))
            .or_default()
            .push(r.name.clone());
    }
    let mut keys: Vec<&String> = groups.keys().collect();
    keys.sort();

    let doc = Doc::new();
    let sched = doc.get_or_insert_map("schedule");
    {
        let mut txn = doc.transact_mut();
        for key in keys {
            let names = &groups[key];
            sched.insert(&mut txn, key.clone(), ArrayPrelim::from(names.clone()));
        }
    }
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    update
}

/// A loaded doc + the broadcast channel that fans updates to its peers.
struct Room {
    doc: Doc,
    tx: broadcast::Sender<(u64, Vec<u8>)>,
}

/// App-scoped collaborative state: one [`Room`] per schedule id.
pub struct CollabState {
    rooms: Mutex<HashMap<i64, Room>>,
    next_conn: AtomicU64,
}

/// What a joining connection needs: its id, a doc handle, the sender to publish
/// on, and a receiver for peers' updates.
pub struct Joined {
    pub conn_id: u64,
    pub doc: Doc,
    pub tx: broadcast::Sender<(u64, Vec<u8>)>,
    pub rx: broadcast::Receiver<(u64, Vec<u8>)>,
}

impl Default for CollabState {
    fn default() -> Self {
        Self::new()
    }
}

impl CollabState {
    pub fn new() -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
        }
    }

    pub fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    /// Join (creating the room if absent). `initial_state` is the persisted
    /// `yjs_state` from the DB, applied only when the room is first created.
    pub fn join(&self, schedule_id: i64, initial_state: Option<&[u8]>) -> Joined {
        let conn_id = self.next_conn_id();
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.entry(schedule_id).or_insert_with(|| {
            let doc = Doc::new();
            doc.get_or_insert_map("schedule");
            if let Some(state) = initial_state {
                if let Ok(update) = Update::decode_v1(state) {
                    let mut txn = doc.transact_mut();
                    let _ = txn.apply_update(update);
                }
            }
            let (tx, _rx) = broadcast::channel(256);
            Room { doc, tx }
        });
        Joined {
            conn_id,
            doc: room.doc.clone(),
            tx: room.tx.clone(),
            rx: room.tx.subscribe(),
        }
    }

    /// A clone of the live doc if a room exists (used by commit to read the
    /// current in-memory state when a session is active).
    pub fn peek_doc(&self, schedule_id: i64) -> Option<Doc> {
        self.rooms
            .lock()
            .unwrap()
            .get(&schedule_id)
            .map(|r| r.doc.clone())
    }

    /// Drop the room when its last subscriber has left (state is already
    /// persisted, so it reloads lazily on demand). Mirrors the collab `finally`
    /// block freeing `rooms`/`docs`.
    pub fn leave_if_empty(&self, schedule_id: i64) {
        let mut rooms = self.rooms.lock().unwrap();
        if let Some(room) = rooms.get(&schedule_id) {
            if room.tx.receiver_count() == 0 {
                rooms.remove(&schedule_id);
            }
        }
    }
}

/// The full v1 update of a doc (the bytes to persist as `yjs_state`).
pub fn full_update(doc: &Doc) -> Vec<u8> {
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}

/// This doc's encoded state vector (server's sync step 1 payload).
pub fn state_vector(doc: &Doc) -> Vec<u8> {
    doc.transact().state_vector().encode_v1()
}

/// The update a peer is missing, given their encoded state vector (sync step 2).
pub fn diff_for(doc: &Doc, client_sv: &[u8]) -> Vec<u8> {
    let sv = StateVector::decode_v1(client_sv).unwrap_or_default();
    doc.transact().encode_state_as_update_v1(&sv)
}

/// Apply a v1 update to the doc. Returns false on a malformed update.
pub fn apply_update(doc: &Doc, payload: &[u8]) -> bool {
    let update = match Update::decode_v1(payload) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let mut txn = doc.transact_mut();
    txn.apply_update(update).is_ok()
}

/// One conflict: a member placed in two boats on the same day.
#[derive(Debug)]
pub struct Conflict {
    pub name: String,
    pub day: String,
    pub boats: Vec<i64>,
}

/// Read the schedule map and return either the resolved `(name, day, boat)`
/// assignments or the detected conflicts. Mirrors the commit endpoint's loop.
pub fn read_assignments(doc: &Doc) -> Result<Vec<(String, String, i64)>, Vec<Conflict>> {
    let sched = doc.get_or_insert_map("schedule");
    let txn = doc.transact();

    let mut seen: HashMap<(String, String), i64> = HashMap::new();
    let mut conflicts: Vec<Conflict> = Vec::new();

    for (key, value) in sched.iter(&txn) {
        let Some(colon) = key.find(':') else { continue };
        let day = key[..colon].to_string();
        let boat: i64 = match key[colon + 1..].parse() {
            Ok(b) => b,
            Err(_) => continue,
        };
        // value should be a Y.Array of name strings.
        let arr = match value.cast::<ArrayRef>() {
            Ok(a) => a,
            Err(_) => continue,
        };
        for item in arr.iter(&txn) {
            let name = match item.to_json(&txn) {
                Any::String(s) => s.to_string(),
                other => format!("{other}"),
            };
            let k = (name.clone(), day.clone());
            if let Some(&prev) = seen.get(&k) {
                conflicts.push(Conflict {
                    name,
                    day: day.clone(),
                    boats: vec![prev, boat],
                });
            } else {
                seen.insert(k, boat);
            }
        }
    }

    if conflicts.is_empty() {
        Ok(seen
            .into_iter()
            .map(|((name, day), boat)| (name, day, boat))
            .collect())
    } else {
        Err(conflicts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for n in [
            0u64,
            1,
            127,
            128,
            255,
            300,
            16384,
            1_000_000,
            u32::MAX as u64,
        ] {
            let mut buf = Vec::new();
            write_varint(n, &mut buf);
            let (got, pos) = read_varint(&buf, 0).unwrap();
            assert_eq!(got, n);
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn varint_matches_known_encoding() {
        // 128 → [0x80, 0x01] in lib0 varint.
        let mut buf = Vec::new();
        write_varint(128, &mut buf);
        assert_eq!(buf, vec![0x80, 0x01]);
    }

    #[test]
    fn parse_roundtrips_encoded_messages() {
        let payload = b"hello world";
        let msg = encode_update_msg(payload);
        let (m, s, p) = parse_message(&msg).unwrap();
        assert_eq!(m, MSG_SYNC);
        assert_eq!(s, SYNC_UPDATE);
        assert_eq!(p, payload);
    }

    #[test]
    fn parse_rejects_truncated() {
        assert!(parse_message(&[0x00]).is_none()); // missing sub_type + buf
    }

    #[test]
    fn doc_from_rows_then_read_back() {
        let rows = vec![
            ScheduleRow {
                day: "Sat".into(),
                boat: 1,
                name: "Alice".into(),
                level: "k".into(),
            },
            ScheduleRow {
                day: "Sat".into(),
                boat: 1,
                name: "Bob".into(),
                level: "k".into(),
            },
            ScheduleRow {
                day: "Sat".into(),
                boat: 2,
                name: "Carol".into(),
                level: "b".into(),
            },
            ScheduleRow {
                day: "Fri".into(),
                boat: 1,
                name: "Alice".into(),
                level: "k".into(),
            },
        ];
        let update = init_doc_update_from_rows(&rows);
        let doc = Doc::new();
        doc.get_or_insert_map("schedule");
        assert!(apply_update(&doc, &update));

        let assignments = read_assignments(&doc).expect("no conflicts");
        assert_eq!(assignments.len(), 4);
        // Sat:1 has Alice+Bob, Sat:2 Carol, Fri:1 Alice — no (name,day) dup.
    }

    #[test]
    fn read_assignments_detects_conflict() {
        // Alice in two Sat boats → conflict on (Alice, Sat).
        let rows = vec![
            ScheduleRow {
                day: "Sat".into(),
                boat: 1,
                name: "Alice".into(),
                level: "k".into(),
            },
            ScheduleRow {
                day: "Sat".into(),
                boat: 2,
                name: "Alice".into(),
                level: "k".into(),
            },
        ];
        let update = init_doc_update_from_rows(&rows);
        let doc = Doc::new();
        doc.get_or_insert_map("schedule");
        apply_update(&doc, &update);
        let conflicts = read_assignments(&doc).unwrap_err();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].name, "Alice");
        assert_eq!(conflicts[0].day, "Sat");
        assert_eq!(conflicts[0].boats.len(), 2);
    }

    #[test]
    fn sync_diff_between_two_docs() {
        // doc A has data; an empty doc's state vector → diff carries everything.
        let rows = vec![ScheduleRow {
            day: "Sat".into(),
            boat: 1,
            name: "Alice".into(),
            level: "k".into(),
        }];
        let a = Doc::new();
        a.get_or_insert_map("schedule");
        apply_update(&a, &init_doc_update_from_rows(&rows));

        let empty = Doc::new();
        empty.get_or_insert_map("schedule");
        let empty_sv = state_vector(&empty);
        let diff = diff_for(&a, &empty_sv);
        assert!(!diff.is_empty());

        // applying the diff to the empty doc reproduces the schedule
        assert!(apply_update(&empty, &diff));
        assert_eq!(read_assignments(&empty).unwrap().len(), 1);
    }
}
