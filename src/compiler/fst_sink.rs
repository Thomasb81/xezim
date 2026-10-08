//! Background writer for FST (GTKWave binary) waveform dumps.
//!
//! The `fst-writer` body writer packs each value change into an in-memory
//! value-change block and periodically compresses and writes it out. Both the
//! packing and the block flush ran on the simulation thread, where they showed
//! up as ~17x the cost of the simulation itself on an unscoped c906 dump.
//!
//! `FstSink` moves the `FstBodyWriter` onto a dedicated thread and feeds it
//! whole timesteps over an mpsc channel. The simulation thread is then left
//! with only change DETECTION (which needs the signal table and so cannot
//! move); rendering, packing, compression and I/O all happen off it.
//!
//! Ordering is preserved because every send happens under the `pending`
//! mutex and the channel is FIFO. `finish()` is a synchronous rendezvous: it
//! shuts the channel and joins, so the FST trailer is on disk before the call
//! returns.
//!
//! Durability. A value-change block lives in memory until it is flushed, and
//! a run that dies without `finish()` (kill -9, OOM, the Ctrl-C backstop)
//! keeps only the flushed blocks: the reader accepts a file without trailer
//! and takes the end time from the last block. So:
//! - the writer flushes a block when it reaches `FST_FLUSH_AT` bytes, and also
//!   once `XEZIM_FST_FLUSH_SECS` (default 2 s) of wall time have passed since
//!   the last flush with data waiting;
//! - a guard thread wakes every `GUARD_TICK` and hands the simulation thread's
//!   partial batch to the writer, so no timestep waits on the simulation
//!   thread for longer than that (and a stuck simulation thread does not hold
//!   back the timesteps it already finished);
//! - on Ctrl-C / SIGTERM that the simulation thread has not taken within
//!   `TAIL_AFTER` (a long computation inside one slot, a blocked DPI call),
//!   the guard closes the dump at the slot time the simulation thread
//!   published (`interrupt::published_sim_time`) and flushes it, holding the
//!   backstop alarm off until the block is on disk.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::interrupt;

use fst_writer::{FstBodyWriter, FstSignalId};
use xezim_core::value::{LogicBit, Value};

/// Timesteps buffered on the simulation thread before a channel send. Batching
/// keeps the per-timestep send cost off the hot path without letting the writer
/// fall so far behind that the queue grows without bound.
const FST_BATCH_FLUSH: usize = 64;

/// One time slot: the timestamp plus every value change at it, still in VALUE
/// form. Rendering to the FST wire form (an ASCII bit string) is the bulk of a
/// dump's per-change cost and happens on the WRITER thread — the simulation
/// thread only clones the `Value`, which for the <=64-bit signals that dominate
/// any real design is an inline 24-byte memcpy with no allocation at all.
pub struct FstTimestep {
    pub time: u64,
    pub changes: Vec<(FstSignalId, Value)>,
}

/// Render a `Value` as the FST bit string: full width, MSB first, '0'/'1'/'x'/'z'.
/// Width-0 yields a single '0' so the writer never sees an empty change.
pub fn fst_format_value(val: &Value) -> Vec<u8> {
    if val.is_real {
        return val.to_f64().to_le_bytes().to_vec();
    }
    let w = val.width as usize;
    if w == 0 {
        return vec![b'0'];
    }
    let mut s = Vec::with_capacity(w);
    for i in (0..w).rev() {
        s.push(match val.get_bit(i) {
            LogicBit::Zero => b'0',
            LogicBit::One => b'1',
            LogicBit::X => b'x',
            LogicBit::Z => b'z',
        });
    }
    s
}

enum Msg {
    Batch(Vec<FstTimestep>),
    /// Force a value-change block out to the OS file so a crash (`panic =
    /// "abort"` skips Drop) leaves a readable partial dump.
    Flush,
    /// Close the dump at `time` (an empty timestep) and flush it, then
    /// signal `done`. Sent by the guard thread for an interrupt the
    /// simulation thread did not take.
    Tail {
        time: u64,
        done: Sender<()>,
    },
    /// Write the FST trailer and stop. The worker exits after this.
    Finish,
}

pub type FstBody = FstBodyWriter<std::io::BufWriter<std::fs::File>>;

/// Crash-safe periodic flush once the in-memory block grows large (matches the
/// fst-writer example's FLUSH_AT).
const FST_FLUSH_AT: usize = 64 * 1024 * 1024;

/// Default for `XEZIM_FST_FLUSH_SECS`.
const FST_FLUSH_SECS_DEFAULT: f64 = 2.0;

/// The guard thread's period: the longest a finished timestep waits in the
/// simulation thread's batch before it is handed to the writer.
const GUARD_TICK: Duration = Duration::from_millis(100);

/// How long the simulation thread has to take an interrupt before the guard
/// thread writes the dump tail itself.
const TAIL_AFTER: Duration = Duration::from_millis(500);

/// Upper bound on the guard's wait for the writer to finish the tail (it may
/// first have to drain a backlog of batches).
const TAIL_WAIT: Duration = Duration::from_secs(30);

/// `XEZIM_FST_FLUSH_SECS`: wall seconds between time-based block flushes;
/// `0` turns the time rule off (blocks are then flushed at `FST_FLUSH_AT`).
fn flush_interval() -> Option<Duration> {
    let secs = std::env::var("XEZIM_FST_FLUSH_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(FST_FLUSH_SECS_DEFAULT);
    (secs.is_finite() && secs > 0.0).then(|| Duration::from_secs_f64(secs))
}

/// The `FstBodyWriter` plus the bookkeeping the flush rules need.
struct Writer {
    body: FstBody,
    /// The dump's path, for the time-table repair after each flush.
    path: Option<String>,
    /// Latest time handed to `time_change`.
    last_time: u64,
    /// The in-memory block has a time-table entry, so a flush writes a real
    /// block. fst-writer cannot take a value change at a time whose block is
    /// already flushed (`todo!`), and a block without time entries is not
    /// worth writing.
    open: bool,
    flushed_once: bool,
    last_flush: Instant,
    every: Option<Duration>,
    /// File offset up to which `repair_time_tables` has checked the blocks.
    repaired_to: u64,
}

impl Writer {
    fn new(body: FstBody, path: Option<String>, start_time: u64) -> Self {
        Writer {
            body,
            path,
            last_time: start_time,
            // The t=start snapshot opened the block's time table unless it
            // was taken at time 0 (the buffer's initial end time).
            open: start_time > 0,
            flushed_once: false,
            last_flush: Instant::now(),
            every: flush_interval(),
            repaired_to: 0,
        }
    }

    fn apply(&mut self, ts: &FstTimestep) {
        if ts.time > self.last_time {
            let _ = self.body.time_change(ts.time);
            self.last_time = ts.time;
            self.open = true;
        } else if self.flushed_once && !self.open {
            // This time's block is already on disk (only after a tail flush,
            // when the simulation thread later finishes the slot it was in);
            // fst-writer cannot add to it.
            return;
        }
        for (fid, val) in &ts.changes {
            let _ = self.body.signal_change(*fid, &fst_format_value(val));
        }
        if self.body.size() >= FST_FLUSH_AT {
            self.flush();
        }
    }

    /// Write the in-memory block out. The block's closing seek empties the
    /// `BufWriter`, so the block is in the OS file when this returns.
    fn flush(&mut self) {
        if !self.open {
            return;
        }
        let _ = self.body.flush();
        self.open = false;
        self.flushed_once = true;
        self.last_flush = Instant::now();
        if let Some(path) = &self.path {
            self.repaired_to = repair_time_tables(path, self.repaired_to);
        }
    }

    /// Time to flush under `XEZIM_FST_FLUSH_SECS`.
    fn due(&self) -> bool {
        self.open
            && self
                .every
                .is_some_and(|every| self.last_flush.elapsed() >= every)
    }

    /// How long the writer thread may block before the time rule needs it.
    fn idle_wait(&self) -> Option<Duration> {
        if !self.open {
            return None;
        }
        self.every
            .map(|every| every.saturating_sub(self.last_flush.elapsed()))
    }

    fn tail(&mut self, time: u64) {
        self.apply(&FstTimestep {
            time: time.max(self.last_time),
            changes: Vec::new(),
        });
        self.flush();
    }

    /// Write the last block and the trailer. `body.finish` drops the
    /// `BufWriter`, so the file is complete when the repair walks the blocks
    /// not checked yet.
    fn finish(self) {
        let _ = self.body.finish();
        if let Some(path) = &self.path {
            repair_time_tables(path, self.repaired_to);
        }
    }
}

/// The simulation thread's batch and the channel to the writer, behind one
/// mutex: whoever sends (the simulation thread on a full batch, the guard on
/// a tick or a tail) sends in order.
struct Pending {
    batch: Vec<FstTimestep>,
    tx: Option<Sender<Msg>>,
}

impl Pending {
    fn hand_off(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let batch = std::mem::replace(&mut self.batch, Vec::with_capacity(FST_BATCH_FLUSH));
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Batch(batch));
        }
    }
}

struct Shared {
    pending: Mutex<Pending>,
    closed: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

enum Mode {
    Inline {
        w: Box<Writer>,
        posts: u32,
    },
    Threaded {
        shared: Arc<Shared>,
        writer: Option<JoinHandle<()>>,
        guard: Option<JoinHandle<()>>,
    },
    Done,
}

pub struct FstSink {
    mode: Mode,
}

fn writer_main(mut w: Writer, rx: mpsc::Receiver<Msg>) {
    loop {
        let msg = match w.idle_wait() {
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => {
                    w.flush();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(m) => m,
                Err(_) => break,
            },
        };
        match msg {
            Msg::Batch(batch) => {
                for ts in &batch {
                    w.apply(ts);
                }
                if w.due() {
                    w.flush();
                }
            }
            Msg::Flush => w.flush(),
            Msg::Tail { time, done } => {
                w.tail(time);
                let _ = done.send(());
            }
            Msg::Finish => break,
        }
    }
    w.finish();
}

fn guard_main(shared: Arc<Shared>) {
    let mut seen: Option<Instant> = None;
    let mut tail_done = false;
    loop {
        std::thread::park_timeout(GUARD_TICK);
        if shared.closed.load(Ordering::Acquire) {
            break;
        }
        shared.lock().hand_off();
        if tail_done || !interrupt::interrupt_requested() {
            continue;
        }
        let since = *seen.get_or_insert_with(Instant::now);
        if interrupt::interrupt_acknowledged() {
            tail_done = true;
        } else if since.elapsed() >= TAIL_AFTER {
            write_tail(&shared, since);
            tail_done = true;
        }
    }
}

/// An interrupt the simulation thread has not taken: close the dump at the
/// published slot time and wait until that block is on disk, with the
/// backstop alarm held off meanwhile.
fn write_tail(shared: &Shared, since: Instant) {
    interrupt::hold_backstop();
    let time = interrupt::published_sim_time();
    let (done_tx, done_rx) = mpsc::channel();
    let sent = {
        let mut p = shared.lock();
        p.hand_off();
        p.tx.as_ref().is_some_and(|tx| {
            tx.send(Msg::Tail {
                time,
                done: done_tx,
            })
            .is_ok()
        })
    };
    if sent {
        eprintln!(
            "[xezim] interrupt not taken within {} ms (the run is inside one time slot) — flushing the FST dump up to time {}",
            TAIL_AFTER.as_millis(),
            time
        );
        if done_rx.recv_timeout(TAIL_WAIT).is_ok() {
            eprintln!("[xezim] FST dump flushed up to time {}", time);
        }
    }
    interrupt::release_backstop(since.elapsed());
}

impl FstSink {
    /// `path` is the dump's file (for the time-table repair after each
    /// flush); `start_time` the time of the initial snapshot already in
    /// `body`.
    pub fn inline(body: FstBody, path: &str, start_time: u64) -> Self {
        FstSink {
            mode: Mode::Inline {
                w: Box::new(Writer::new(body, Some(path.to_string()), start_time)),
                posts: 0,
            },
        }
    }

    pub fn threaded(body: FstBody, path: &str, start_time: u64) -> Self {
        let w = Writer::new(body, Some(path.to_string()), start_time);
        let (tx, rx) = mpsc::channel::<Msg>();
        let writer = std::thread::Builder::new()
            .name("xezim-fst".to_string())
            .spawn(move || writer_main(w, rx))
            .expect("spawn xezim-fst writer thread");
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending {
                batch: Vec::with_capacity(FST_BATCH_FLUSH),
                tx: Some(tx),
            }),
            closed: AtomicBool::new(false),
        });
        let s2 = Arc::clone(&shared);
        // Without the guard the dump still works; it only loses the
        // age-based hand-off and the interrupt tail.
        let guard = std::thread::Builder::new()
            .name("xezim-fst-guard".to_string())
            .spawn(move || guard_main(s2))
            .ok();
        FstSink {
            mode: Mode::Threaded {
                shared,
                writer: Some(writer),
                guard,
            },
        }
    }

    pub fn post(&mut self, ts: FstTimestep) {
        match &mut self.mode {
            Mode::Inline { w, posts } => {
                w.apply(&ts);
                // The clock is read once every 16 timesteps only.
                *posts = posts.wrapping_add(1);
                if *posts % 16 == 0 && w.due() {
                    w.flush();
                }
            }
            Mode::Threaded { shared, .. } => {
                let mut p = shared.lock();
                p.batch.push(ts);
                if p.batch.len() >= FST_BATCH_FLUSH {
                    p.hand_off();
                }
            }
            Mode::Done => {}
        }
    }

    /// Durable flush: push everything buffered here to the worker and have it
    /// write a value-change block out.
    pub fn flush(&mut self) {
        match &mut self.mode {
            Mode::Inline { w, .. } => w.flush(),
            Mode::Threaded { shared, .. } => {
                let mut p = shared.lock();
                p.hand_off();
                if let Some(tx) = &p.tx {
                    let _ = tx.send(Msg::Flush);
                }
            }
            Mode::Done => {}
        }
    }

    /// Write the FST trailer and close the file. Blocks until the worker has
    /// finished, so the dump is complete and readable when this returns.
    pub fn finish(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        match std::mem::replace(&mut self.mode, Mode::Done) {
            Mode::Inline { w, .. } => w.finish(),
            Mode::Threaded {
                shared,
                writer,
                guard,
            } => {
                shared.closed.store(true, Ordering::Release);
                {
                    let mut p = shared.lock();
                    p.hand_off();
                    if let Some(tx) = p.tx.take() {
                        let _ = tx.send(Msg::Finish);
                    }
                    // Dropping the sender also ends the worker's `recv` if
                    // the Finish message were ever lost.
                }
                if let Some(g) = guard {
                    g.thread().unpark();
                    let _ = g.join();
                }
                if let Some(h) = writer {
                    let _ = h.join();
                }
            }
            Mode::Done => {}
        }
    }
}

impl Drop for FstSink {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// Undo `fst-writer`'s break-even time-table encoding in the value-change
/// blocks from file offset `from` on (a block boundary: 0, or a value this
/// function returned). Returns the offset just past the last complete block
/// it walked, so a caller can check each block once as the file grows.
///
/// `write_time_table` picks raw vs zlib storage with `compressed.len() >
/// raw.len()`. The format needs `>=`: a reader treats "compressed length ==
/// uncompressed length" as the sentinel for a section stored RAW, so when
/// zlib output comes out EXACTLY the size of its input the writer emits
/// compressed bytes while recording equal lengths. Readers then skip the
/// inflate and parse zlib's own header as varints — the first two
/// timestamps decode from the `78 5e` magic as 120 and 214 for any design.
///
/// Nothing else about such a file is wrong: header, block chain, GEOM and
/// the declared start/end times all validate, so it passes every structural
/// check and only the per-change times are nonsense. gtkwave's `fst2vcd`
/// "succeeds" on one and prints times orders of magnitude off, which reads
/// as a simulator timing bug rather than a dump bug.
///
/// Break-even needs only a short run whose delta-encoded table is small and
/// incompressible — 19 irregular time steps is enough — so this is not a
/// rare corner. Repairing here keeps the fix inside xezim instead of
/// carrying a patched copy of the crate.
///
/// Best-effort by construction: every failure path leaves the file exactly
/// as the writer left it, because a dump that is merely mis-flagged is far
/// better than one this pass half-rewrote. A repaired block no longer
/// matches the break-even pattern, so running it twice is harmless.
pub fn repair_time_tables(path: &str, from: u64) -> u64 {
    use std::io::{Read, Seek, SeekFrom, Write};
    const ZLIB_MAGIC: [[u8; 2]; 4] = [[0x78, 0x9c], [0x78, 0x5e], [0x78, 0x01], [0x78, 0xda]];
    // Value-change block types that carry a trailing time table.
    const VC_TYPES: [u8; 3] = [1, 5, 8];

    let Ok(mut f) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    else {
        return from;
    };
    let Ok(size) = f.metadata().map(|m| m.len()) else {
        return from;
    };
    let mut off: u64 = from;
    while off < size {
        if f.seek(SeekFrom::Start(off)).is_err() {
            break;
        }
        let mut head = [0u8; 9];
        if f.read_exact(&mut head).is_err() {
            break;
        }
        let btype = head[0];
        let blen = u64::from_be_bytes(head[1..9].try_into().unwrap_or([0; 8]));
        if blen == 0 || off + 1 + blen > size {
            break; // malformed or still being written: leave it alone
        }
        let end = off + 1 + blen;
        if VC_TYPES.contains(&btype) {
            // Trailer is (uncompressed_len, compressed_len, item_count).
            let mut tr = [0u8; 24];
            if f.seek(SeekFrom::Start(end - 24)).is_ok() && f.read_exact(&mut tr).is_ok() {
                let unc = u64::from_be_bytes(tr[0..8].try_into().unwrap_or([0; 8]));
                let comp = u64::from_be_bytes(tr[8..16].try_into().unwrap_or([0; 8]));
                if unc == comp && comp >= 2 && comp <= blen {
                    let start = end - 24 - comp;
                    let mut payload = vec![0u8; comp as usize];
                    if f.seek(SeekFrom::Start(start)).is_ok()
                        && f.read_exact(&mut payload).is_ok()
                        && ZLIB_MAGIC.iter().any(|m| payload[..2] == *m)
                    {
                        if let Ok(raw) = miniz_oxide::inflate::decompress_to_vec_zlib(&payload) {
                            if raw.len() as u64 == comp {
                                // The break-even case itself: the lengths are
                                // LEGITIMATELY equal, so correcting a length
                                // would change nothing. Store what should have
                                // been stored. Same byte count, so no offset in
                                // the file moves.
                                let _ = f
                                    .seek(SeekFrom::Start(start))
                                    .and_then(|_| f.write_all(&raw));
                            } else {
                                // They only looked equal; recording the true
                                // uncompressed length makes the reader inflate.
                                let _ = f
                                    .seek(SeekFrom::Start(end - 24))
                                    .and_then(|_| f.write_all(&(raw.len() as u64).to_be_bytes()));
                            }
                        }
                    }
                }
            }
        }
        off = end;
    }
    let _ = f.flush();
    off
}
