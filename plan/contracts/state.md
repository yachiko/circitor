# Seam: the state store and the state file

**Purpose.** The one place the tracked PRs, their records, the revision counter and the event history live (STA-1 to STA-5, STA-9, STA-10, MOD-13); and `state.json` (STA-6 to STA-8). The engine writes to the store, the API and the waiters read it, the saver turns it into the file.

**Producer:** T02 declares every type and signature below, with `unimplemented!()` bodies. T15 builds `state::store`; T16 builds `state::file`. **Consumers:** T21, T25 (the engine), T22 (routes), T23 (waiters), T28 (wiring), T29.

## Declarations (`src/state/mod.rs`, shared)

```rust
#[derive(Clone, Copy, Debug, thiserror::Error)] #[error("limit")] pub struct Limit;   // STA-3's 200, API-14's 64
pub const MAX_PLACES: usize = 200;        // STA-3
pub const MAX_WAITERS: u32 = 64;          // API-14
pub const MAX_EVENTS: usize = 500;        // STA-10; also `limit`'s range (API-8)

/// STA-9. `Display` is `<epoch>-<rev>`; `FromStr` takes exactly that (8 of `[a-z0-9]`, `-`, digits) or fails.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct Cursor { pub epoch: String, pub rev: u64 }

/// What `state.json` holds, version 1 (STA-6), without its `version`: reading and writing that key,
/// and the object around `prs`, are `state::file`'s own (T16). Serde names are the file's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileContent { pub prs: Vec<FileEntry> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub pr: PrName, pub id: NodeId,
    pub source: Source,                       // never `asked`
    pub login: Option<String>,
    pub ended: Option<Ended>,
    pub seen: Seen,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ended { pub state: PrState, pub seen_at: Time }   // `merged`, `closed` or `gone`

/// What the saver reads (T16), and the store provides (T15).
pub trait FileSource: Send + Sync + 'static {
    fn file_content(&self) -> FileContent;
    /// Changes whenever `file_content()` would return something else.
    fn file_changed(&self) -> tokio::sync::watch::Receiver<u64>;
}
/// What the routes call after a `track` or an `untrack` (STA-7). T16's saver implements it.
#[async_trait::async_trait]
pub trait Flush: Send + Sync + 'static {
    /// Writes now if the content changed since the last write, whatever the 5 s spacing, and returns
    /// when the write succeeded or failed. A failure is the saver's to report and retry.
    async fn flush(&self);
}
pub struct NoFlush;                           // the fake: does nothing
```

## Declarations (`src/state/store/`)

```rust
pub struct StoreDeps { pub clock: SharedClock, pub logger: Logger, pub config: Arc<Config>, pub seed: u64 }

/// Why a tracked PR is where it is. `Loaded` and `First` have no record yet: they're API-18's `pending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class { Loaded, First, Open, Ended }

/// What the store knows about a PR's refreshes, for cadence and staleness.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timing {
    pub last_sent: Option<Mono>,              // its last refresh of any kind, when sent (POL-13)
    pub fetched: Option<Mono>,                // the send time of the read last applied
    pub attempt_failed: bool,                 // MOD-16: its last attempt failed
    pub slept: bool,                          // MOD-16: a sleep was noticed since it was fetched
    pub waiters: u32,
    pub named: Option<Mono>,                  // when a request last named it (POL-14, POL-11)
}
#[derive(Clone, Debug)]
pub struct Target {
    pub id: NodeId, pub pr: PrName, pub source: Source, pub class: Class,
    pub status: Option<Status>,
    pub mergeable_unknown: bool,              // the read last applied said `UNKNOWN` (POL-11), whatever MOD-8 kept in the record
    pub updated_at: Option<Time>, pub timing: Timing,
}
#[derive(Clone, Debug)]
pub struct Tracked { pub id: NodeId, pub class: Class, pub source: Source }

#[derive(Clone, Debug)]
pub struct PrView { pub id: NodeId, pub record: Arc<Record>, pub timing: Timing }
/// One consistent view (MOD-13): every answer is built from one of these.
#[derive(Clone, Debug)]
pub struct View {
    pub epoch: String,
    pub rev: u64,                             // the counter: the highest revision taken
    pub prs: Vec<PrView>,                     // every PR with a record, in no promised order
    pub events: Arc<[Event]>,                 // the history held, oldest first (STA-10)
    pub pending: Vec<PrName>,                 // `Loaded` and `First`, sorted
    pub tracked: [u32; 3],                    // counts by source: authored, manual, asked (API-5)
    pub waiters: u32,
}
impl View {                                   // built by T02: pure over the public fields above (T43's `window` uses them)
    pub fn cursor(&self) -> Cursor;
    /// STA-9: another epoch, a `rev` above the counter, or an event after it no longer held.
    pub fn too_old(&self, c: &Cursor) -> bool;
    pub fn find(&self, name: &PrName) -> Option<&PrView>;
    pub fn get(&self, id: &NodeId) -> Option<&PrView>;
    /// Events with `rev` above `after`, oldest first; for one PR when `id` is given.
    pub fn events_after(&self, after: u64, id: Option<&NodeId>) -> Vec<&Event>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum Applied { Changed { rev: u64 }, Unchanged, Discarded }
#[derive(Clone, Debug)]
pub struct Untracked { pub record: Option<Record>, pub file_changed: bool }

pub struct Store { /* a std::sync::Mutex inside; never held across an await */ }
impl Store {
    /// A new epoch (STA-9). `loaded` entries are tracked without a record (STA-8); those past
    /// `keep_done` are dropped without an event; an `authored` one outside `owners` becomes `asked` (POL-5).
    pub fn new(deps: StoreDeps, loaded: FileContent) -> Arc<Store>;

    // ---- the tracked set ----
    pub fn lookup(&self, name: &PrName) -> Option<Tracked>;
    pub fn is_tracked(&self, id: &NodeId) -> bool;
    pub fn places_free(&self) -> usize;                       // 200 minus every place taken (STA-3)
    /// Takes a place for a PR found (`Authored`) or read through (`Asked`, or `Manual` for `track`).
    /// For a tracked ID: no new place, the stronger source is kept (STA-1), `Ok`.
    pub fn reserve(&self, id: &NodeId, name: &PrName, source: Source) -> Result<(), Limit>;
    /// Gives back the place of a `First` PR whose first refresh failed (STA-3). Nothing for any other.
    pub fn release(&self, id: &NodeId);
    /// POL-5: these PRs become `asked`. The engine passes the `authored` PRs whose refresh **this poll
    /// applied and showed open**, and which this poll's finding, after the owners filter and read to its
    /// last page, didn't list. The store changes only those that are `authored` and open; never one the
    /// poll didn't refresh (a PR merged since the last poll is unlisted and still reads open until then).
    pub fn demote_unlisted(&self, unlisted_open: &[NodeId]);
    /// API-15 `track`: source `manual`. `true` if the file's content changed. On a `Loaded` entry it
    /// changes the saved source: the one change a saved entry takes before its first refresh (STA-8).
    pub fn set_manual(&self, id: &NodeId) -> bool;
    /// API-15 `untrack`, whole: `authored`, `asked` (with a waiter), dropped with an `untracked` event, or nothing.
    pub fn untrack(&self, name: &PrName) -> Untracked;

    // ---- refreshes (the engine) ----
    pub fn targets(&self) -> Vec<Target>;
    /// `Timing::last_sent`. For a poll's requests `at` is the poll's own send time (`poller.md`).
    pub fn mark_sent(&self, ids: &[NodeId], at: Mono);
    pub fn counts_rose(&self, id: &NodeId, read: &PrRead) -> bool;    // POL-12; `false` without a record
    /// STA-4. `login` is the poll's `viewer.login`, or `None` outside a poll (kept as it was).
    pub fn apply(&self, id: &NodeId, read: &PrRead, items: ItemsOutcome<'_>, meta: &ReadMeta, login: Option<&str>) -> Applied;
    /// A refresh of this PR failed (POL-19, POL-21, POL-24, POL-8): it's `stale`. With `error_type`
    /// the record's `error` is set (MOD-2); without one it's left as it was.
    pub fn attempt_failed(&self, id: &NodeId, error_type: Option<&str>, at: Time);
    pub fn login_of(&self, id: &NodeId) -> Option<String>;            // MOD-11's "same login"
    /// POL-21: an open PR becomes `gone`, with its event; its retention clock starts at `seen_at` (STA-2).
    /// A `Loaded` entry saved open has no record to change: its saved entry becomes `gone` from
    /// `seen_at`, with no event, and this returns `Unchanged`.
    pub fn mark_gone(&self, id: &NodeId, seen_at: Time) -> Applied;
    /// POL-22: every record is `stale` until its next refresh.
    pub fn mark_slept(&self);
    /// The tick (POL-26): drops what STA-1's hour and STA-2's `keep_done` say, by wall time, in the
    /// order of `pr`, each with an `untracked` event. Never a PR with a waiter.
    pub fn sweep(&self);

    // ---- readers (the API, the waiters) ----
    pub fn view(&self) -> View;
    /// Changes after every `apply` (changed or not), `attempt_failed`, `mark_gone`, `sweep` that
    /// dropped something, `untrack`, `set_manual` and `demote_unlisted`. Waiters evaluate on it (STA-4).
    pub fn changed(&self) -> tokio::sync::watch::Receiver<u64>;
    /// A request names this PR (STA-1): `Timing::named` is now. Dropping the guard is the request's end,
    /// from which an `asked` PR's 60 minutes run.
    pub fn name(&self, id: &NodeId) -> Named;
    /// A waiter on this PR (API-9, API-14: at most 64 in all). Dropping the guard removes it, and
    /// carries out a drop that fell due meanwhile (STA-1).
    pub fn add_waiter(&self, id: &NodeId) -> Result<Waiting, Limit>;
    /// When the last waiter ended; `None` if there never was one (OPS-8's "no waiter for `exit_after`").
    pub fn last_waiter_ended(&self) -> Option<Mono>;
}
pub struct Named(/* … */);  pub struct Waiting(/* … */);     // guards; `Drop` does the work
// impl FileSource for Store
```

## Declarations (`src/state/file/`)

```rust
/// Every file operation of STA-7, one call per step, so a test can stop after any of them (TST-10).
pub trait StateFs: Send + Sync + 'static {
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>>;      // `None`: no such file
    fn write_tmp(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>; // create or replace, mode 0600
    fn sync_file(&self, path: &Path) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()>;
}
pub struct RealFs;

pub enum Loaded {
    Content(FileContent),          // also for a missing file: empty
    SetAside,                      // not JSON or no integer `version`: renamed to `state.json.bad`; start empty
    Newer,                         // a newer `version`: left alone, never written; start empty
}
/// STA-8. Skips and logs entries that don't decode; raises `state_file_set_aside` or `state_file_newer`.
pub fn load(fs: &dyn StateFs, path: &Path, logger: &Logger, warnings: &Warnings) -> Loaded;

pub struct SaverDeps { pub fs: Arc<dyn StateFs>, pub path: PathBuf, pub source: Arc<dyn FileSource>,
                       pub clock: SharedClock, pub logger: Logger, pub warnings: Warnings,
                       pub activity: Activity, pub writable: bool /* false after `Loaded::Newer` */ }
pub struct Saver { /* … */ }
impl Saver {
    /// Starts the task that writes when the content changes, at most once in 5 s, and retries a failed
    /// write at the next change or 60 s later (STA-7).
    pub fn spawn(deps: SaverDeps) -> Arc<Saver>;
    /// OPS-7: the final write, without the spacing. `Err` if it failed. Stops the task.
    pub async fn shutdown(&self) -> std::io::Result<()>;
    /// Stops the task without writing (`Running::crash`, TST-5).
    pub fn abort(&self);
}
// impl Flush for Saver
```

## Semantics

- **One lock, short.** Every `Store` method takes the lock, does its work and returns. `view()` clones `Arc`s. Nothing awaits inside.
- **A revision, its record and its event are stored in one step (MOD-13).** A `View` never shows a counter above its newest event, or a record whose `rev` has no event (while history holds it).
- **`apply`** picks `model::Before` itself (`First` is `New`, `Loaded` is `Loaded`, else `Known`), calls `model::apply_read`, and: `Discarded` if the PR holds no place or the read is older (STA-4, MOD-2); else stores the record and carry, clears `attempt_failed`, `slept` and `error`, sets `Timing::fetched`; `Changed` takes the next revision and records the event; `Unchanged` doesn't. It starts or clears the retention clock (STA-2), and logs tracked and gone (OPS-9).
- **A place** is held by every tracked PR of any class (STA-3). `reserve` on a name that's `Loaded` under another ID doesn't happen: IDs are the key, names are looked up first (`lookup`).
- **The same PR under a new name (MOD-1).** `apply` takes `read.pr` as the name; `lookup` finds a record by its current name, and a `Loaded` entry by its saved one.
- **`Untracked::record`** for a PR that was dropped is its last record, with the `rev` of its `untracked` event; it has no `Timing`, so the route serves it as it is (`stale: false`).
- **An `asked` PR's 60 minutes** (STA-1) start whenever a PR becomes `asked` (a read-through, `untrack` with a waiter, `demote_unlisted`), and start again at each `Named` drop.
- **`untrack` and `set_manual` never call GitHub** and never wait. The route calls `Flush::flush` afterwards when `file_changed` (STA-7).
- **`sweep` and waiters (STA-1, STA-2).** A drop that falls due while a PR has a waiter waits for `Waiting`'s `Drop`. The refresh that ends a PR is applied, and `changed()` moved, before any drop, also with `keep_done` 0: `sweep` runs at the tick, never inside `apply`.
- **`View::too_old`** is judged on the view the answer is built from (STA-9), so a waiter calls it again when it answers.
- **The saver** reads `FileSource` only. It writes `state.json.tmp`, syncs it, renames it over `state.json`, syncs the directory (STA-7), through `StateFs`. A failed write raises `state_file_unwritable`; a good one clears it. With `writable: false` it never writes (STA-8).
- **Order at start (STA-11, T28):** `load`, then `Store::new(deps, content)`, then `Saver::spawn`.

## The fakes

- `NoFlush` for route tests (T02).
- `state::file::FaultFs` (T16): an in-memory `StateFs` that can fail or stop after any step, and play a power cut that loses what wasn't synced (TST-10). The harness can hand it to the daemon.
- There is no fake `Store`: T21, T22 and T23 depend on T15 and use the real one, filled through `reserve` and `apply` with reads from `github::fake::answers`.
- `testdata/state/` is T16's: `v1.json` (STA-6's example with an unknown key added, one entry per source and per `ended` state) and the bad ones.

## Don't assume

- That `Record.stale` is set in a `PrView`: compute it with `poller::cadence::stale(&view.record, &view.timing, …)`.
- That `View.prs` is sorted: API-6's order is the route's.
- That an `asked` PR is in `file_content()`: it never is (STA-6).
