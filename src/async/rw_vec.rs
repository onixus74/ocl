//! A mutex-like lock which can be shared between threads and can interact
//! with OpenCL events.
//!
//!
//! TODO: Add doc links.
//
//

extern crate qutex;

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use futures::{Future, Poll, Async};
use futures::sync::oneshot::{self, Receiver};
use core::ClContextPtr;
use ::{Event, EventList};
use async::{Error as AsyncError, Result as AsyncResult};
// pub use self::qutex::{Request, Guard, FutureGuard, Qutex};
pub use self::qutex::{ReadGuard as QrwReadGuard, WriteGuard as QrwWriteGuard,
    FutureReadGuard as QrwFutureReadGuard, FutureWriteGuard as QrwFutureWriteGuard, QrwLock,
    QrwRequest, RequestKind};

const PRINT_DEBUG: bool = false;

pub type FutureReadGuard<T> = FutureRwGuard<T, ReadGuard<T>>;
pub type FutureWriteGuard<T> = FutureRwGuard<T, WriteGuard<T>>;


fn print_debug(id: usize, msg: &str) {
    if PRINT_DEBUG {
        println!("###### [{}] {} (thread: {})", id, msg,
            ::std::thread::current().name().unwrap_or("<unnamed>"));
    }
}

// /// Extracts an `RwVec` from a guard of either type.
// //
// // This saves us two unnecessary atomic stores (the reference count of lock
// // going up then down when releasing or up/downgrading) which would occur if
// // we were to clone then drop.
// unsafe fn extract_rw_vec<T, G: Guard<T>>(guard: G) -> QrwLock<T> {
//     let rw_vec = ::std::ptr::read(guard.lock());
//     ::std::mem::forget(guard);
//     rw_vec
// }


/// A read or write guard for an `RwVec`.
pub trait RwGuard<T> {
    fn new(rw_vec: RwVec<T>, release_event: Option<Event>) -> Self;
}

/// Allows access to the data contained within a lock just like a mutex guard.
#[derive(Debug)]
pub struct ReadGuard<T> {
    rw_vec: RwVec<T>,
    release_event: Option<Event>,
}

impl<T> ReadGuard<T> {
    /// Returns a new `ReadGuard`.
    fn new(rw_vec: RwVec<T>, release_event: Option<Event>) -> ReadGuard<T> {
        print_debug(rw_vec.id(), "ReadGuard::new: read lock acquired");
        ReadGuard {
            rw_vec: rw_vec,
            release_event: release_event,
        }
    }

    /// Triggers the release event and releases the lock held by this `ReadGuard`
    /// before returning the original `RwVec`.
    //
    // * NOTE: This could be done without refcount incr/decr (see `qrw_lock::extract_lock`).
    pub fn release(guard: ReadGuard<T>) -> RwVec<T> {
        print_debug(guard.rw_vec.id(), "releasing read lock");
        guard.rw_vec.clone()
    }

    /// Returns a reference to the event previously set using
    /// `create_release_event` on the `FutureReadGuard` which preceded this
    /// `ReadGuard`. The event can be manually 'triggered' by calling
    /// `...set_complete()...` or used normally (as a wait event) by
    /// subsequent commands. If the event is not manually completed it will be
    /// automatically set complete when this `ReadGuard` is dropped.
    pub fn release_event(guard: &ReadGuard<T>) -> Option<&Event> {
        guard.release_event.as_ref()
    }

    /// Triggers the release event by setting it complete.
    fn complete_release_event(guard: &ReadGuard<T>) {
        if let Some(ref e) = guard.release_event {
            if !e.is_complete().expect("ReadCompletion::drop") {
                print_debug(guard.rw_vec.id(), "ReadGuard::complete_release_event: \
                    setting release event complete");
                e.set_complete().expect("ReadCompletion::drop");
            }
        }
    }
}

impl<T> Deref for ReadGuard<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        unsafe { &*self.rw_vec.lock.as_ptr() }
    }
}

impl<T> Drop for ReadGuard<T> {
    fn drop(&mut self) {
        print_debug(self.rw_vec.id(), "dropping and releasing ReadGuard");
        unsafe { self.rw_vec.lock.release_read_lock() };
        Self::complete_release_event(self);
    }
}

impl<T> RwGuard<T> for ReadGuard<T> {
    fn new(rw_vec: RwVec<T>, release_event: Option<Event>) -> ReadGuard<T> {
        ReadGuard::new(rw_vec, release_event)
    }
}


/// Allows access to the data contained within just like a mutex guard.
#[derive(Debug)]
pub struct WriteGuard<T> {
    rw_vec: RwVec<T>,
    release_event: Option<Event>,
}

impl<T> WriteGuard<T> {
    /// Returns a new `WriteGuard`.
    fn new(rw_vec: RwVec<T>, release_event: Option<Event>) -> WriteGuard<T> {
        print_debug(rw_vec.id(), "WriteGuard::new: Write lock acquired");
        WriteGuard {
            rw_vec: rw_vec,
            release_event: release_event,
        }
    }

    /// Triggers the release event and releases the lock held by this `WriteGuard`
    /// before returning the original `RwVec`.
    //
    // * NOTE: This could be done without refcount incr/decr (see `qrw_lock::extract_lock`).
    pub fn release(guard: WriteGuard<T>) -> RwVec<T> {
        print_debug(guard.rw_vec.id(), "WriteGuard::release: Releasing write lock");
        guard.rw_vec.clone()
    }

    /// Returns a reference to the event previously set using
    /// `create_release_event` on the `FutureWriteGuard` which preceded this
    /// `WriteGuard`. The event can be manually 'triggered' by calling
    /// `...set_complete()...` or used normally (as a wait event) by
    /// subsequent commands. If the event is not manually completed it will be
    /// automatically set complete when this `WriteGuard` is dropped.
    pub fn release_event(guard: &WriteGuard<T>) -> Option<&Event> {
        guard.release_event.as_ref()
    }

    /// Triggers the release event by setting it complete.
    fn complete_release_event(guard: &WriteGuard<T>) {
        if let Some(ref e) = guard.release_event {
            if !e.is_complete().expect("WriteGuard::complete_release_event") {
                print_debug(guard.rw_vec.id(), "WriteGuard::complete_release_event: \
                    Setting release event complete");
                e.set_complete().expect("WriteGuard::complete_release_event");
            }
        }
    }
}

impl<T> Deref for WriteGuard<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        unsafe { &*self.rw_vec.lock.as_ptr() }
    }
}

impl<T> DerefMut for WriteGuard<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        unsafe { &mut *self.rw_vec.lock.as_mut_ptr() }
    }
}

impl<T> Drop for WriteGuard<T> {
    fn drop(&mut self) {
        print_debug(self.rw_vec.id(), "WriteGuard::drop: Dropping and releasing WriteGuard");
        unsafe { self.rw_vec.lock.release_write_lock() };
        Self::complete_release_event(self);
    }
}

impl<T> RwGuard<T> for WriteGuard<T> {
    fn new(rw_vec: RwVec<T>, release_event: Option<Event>) -> WriteGuard<T> {
        WriteGuard::new(rw_vec, release_event)
    }
}


/// The polling stage of a `FutureRwGuard`.
#[derive(Debug, PartialEq)]
enum Stage {
    Marker,
    QrwLock,
    Command,
    Upgrade,
}


/// A future that resolves to a read or write guard after ensuring that the
/// data being guarded is appropriately locked during the execution of an
/// OpenCL command.
///
/// 1. Waits until both an exclusive data lock can be obtained **and** all
///    prerequisite OpenCL commands have completed.
/// 2. Triggers an OpenCL command, remaining locked while the command
///    executes.
/// 3. Returns a guard which provides exclusive (write) or shared (read)
///    access to the locked data.
///
#[must_use = "futures do nothing unless polled"]
#[derive(Debug)]
pub struct FutureRwGuard<T, G> {
    rw_vec: Option<RwVec<T>>,
    lock_rx: Option<Receiver<()>>,
    wait_list: Option<EventList>,
    lock_event: Option<Event>,
    command_completion: Option<Event>,
    upgrade_after_command: bool,
    upgrade_rx: Option<Receiver<()>>,
    release_event: Option<Event>,
    stage: Stage,
    _guard: PhantomData<G>,
}

impl<T, G> FutureRwGuard<T, G> where G: RwGuard<T> {
    /// Returns a new `FutureRwGuard`.
    fn new(rw_vec: RwVec<T>, lock_rx: Receiver<()>) -> FutureRwGuard<T, G> {
        FutureRwGuard {
            rw_vec: Some(rw_vec),
            lock_rx: Some(lock_rx),
            wait_list: None,
            lock_event: None,
            command_completion: None,
            upgrade_after_command: false,
            upgrade_rx: None,
            release_event: None,
            stage: Stage::Marker,
            _guard: PhantomData,
        }
    }

    /// Sets an event wait list.
    ///
    /// Setting a wait list will cause this `FutureRwGuard` to wait until
    /// contained events have their status set to complete before obtaining a
    /// lock on the guarded internal `Vec`.
    ///
    /// [UNSTABLE]: This method may be renamed or otherwise changed at any time.
    pub fn set_wait_list<L: Into<EventList>>(&mut self, wait_list: L) {
        assert!(self.wait_list.is_none(), "Wait list has already been set.");
        self.wait_list = Some(wait_list.into());
    }

    /// Sets a command completion event.
    ///
    /// If a command completion event corresponding to the read or write
    /// command being executed in association with this `FutureRwGuard` is
    /// specified before this `FutureRwGuard` is polled it will cause this
    /// `FutureRwGuard` to suffix itself with an additional future that will
    /// wait until the command completion event completes before resolving
    /// into an `RwGuard`.
    ///
    /// Not specifying a command completion event will cause this
    /// `FutureRwGuard` to resolve into an `RwGuard` immediately after the
    /// lock is obtained (indicated by the optionally created lock event).
    ///
    /// TODO: Reword this.
    /// [UNSTABLE]: This method may be renamed or otherwise changed at any time.
    pub fn set_command_completion_event(&mut self, command_completion: Event) {
        assert!(self.command_completion.is_none(), "Command completion event has already been set.");
        self.command_completion = Some(command_completion);
    }

    /// Creates an event which will be triggered when a lock is obtained on
    /// the guarded internal `Vec`.
    ///
    /// The returned event can be added to the wait list of subsequent OpenCL
    /// commands with the expectation that when all preceding futures are
    /// complete, the event will automatically be 'triggered' by having its
    /// status set to complete, causing those commands to execute. This can be
    /// used to inject host side code in amongst OpenCL commands without
    /// thread blocking or extra delays of any kind.
    pub fn create_lock_event<C: ClContextPtr>(&mut self, context: C) -> AsyncResult<&Event> {
        assert!(self.lock_event.is_none(), "Lock event has already been created.");
        self.lock_event = Some(Event::user(context)?);
        Ok(self.lock_event.as_mut().unwrap())
    }

    /// Creates an event which will be triggered after this future resolves
    /// **and** the ensuing `RwGuard` is dropped or manually released.
    ///
    /// The returned event can be added to the wait list of subsequent OpenCL
    /// commands with the expectation that when all preceding futures are
    /// complete, the event will automatically be 'triggered' by having its
    /// status set to complete, causing those commands to execute. This can be
    /// used to inject host side code in amongst OpenCL commands without
    /// thread blocking or extra delays of any kind.
    pub fn create_release_event<C: ClContextPtr>(&mut self, context: C) -> AsyncResult<&Event> {
        assert!(self.release_event.is_none(), "Release event has already been created.");
        self.release_event = Some(Event::user(context)?);
        Ok(self.release_event.as_ref().unwrap())
    }

    /// Returns a reference to the event previously created with
    /// `::create_lock_event` which will trigger (be completed) when the wait
    /// events are complete and the lock is locked.
    pub fn lock_event(&self) -> Option<&Event> {
        self.lock_event.as_ref()
    }

    /// Returns a reference to the event previously created with
    /// `::create_release_event` which will trigger (be completed) when a lock
    /// is obtained on the guarded internal `Vec`.
    pub fn release_event(&self) -> Option<&Event> {
        self.release_event.as_ref()
    }

    /// Blocks the current thread until the OpenCL command is complete and an
    /// appropriate lock can be obtained on the underlying data.
    pub fn wait(self) -> AsyncResult<G> {
        <Self as Future>::wait(self)
    }

    /// Returns a mutable pointer to the data contained within the internal
    /// `Vec`, bypassing all locks and protections.
    pub unsafe fn as_mut_ptr(&self) -> Option<*mut T> {
        self.rw_vec.as_ref().map(|rw_vec| (*rw_vec.lock.as_mut_ptr()).as_mut_ptr())
    }

    /// Returns a mutable slice to the data contained within the internal
    /// `Vec`, bypassing all locks and protections.
    pub unsafe fn as_mut_slice<'a, 'b>(&'a self) -> Option<&'b mut [T]> {
        self.as_mut_ptr().map(|ptr| {
            ::std::slice::from_raw_parts_mut(ptr, self.len())
        })
    }

    /// Returns the length of the internal `Vec`.
    pub fn len(&self) -> usize {
        unsafe { (*self.rw_vec.as_ref().expect("FutureRwGuard::len: No RwVec found.")
            .lock.as_ptr()).len() }
    }

    /// The 'id' of the associated `RwVec`.
    pub fn id(&self) -> usize {
        self.rw_vec.as_ref().expect("FutureRwGuard::id: No RwVec found.").id()
    }

    /// Polls the wait events until all requisite commands have completed then
    /// polls the lock queue.
    fn poll_wait_events(&mut self) -> AsyncResult<Async<G>> {
        debug_assert!(self.stage == Stage::Marker);
        print_debug(self.rw_vec.as_ref().unwrap().id(), "FutureRwGuard::poll_wait_events: Called");

        // Check completion of wait list, if it exists:
        if let Some(ref mut wait_list) = self.wait_list {
            // if PRINT_DEBUG { println!("###### [{}] FutureRwGuard::poll_wait_events: \
            //     Polling wait_events (thread: {})...", self.rw_vec.as_ref().unwrap().id(),
            //     ::std::thread::current().name().unwrap_or("<unnamed>")); }

            if let Async::NotReady = wait_list.poll()? {
                return Ok(Async::NotReady);
            }

        }

        self.stage = Stage::QrwLock;
        self.poll_lock()
    }

    /// Polls the lock until we have obtained a lock then polls the command
    /// event.
    #[cfg(not(feature = "async_block"))]
    fn poll_lock(&mut self) -> AsyncResult<Async<G>> {
        debug_assert!(self.stage == Stage::QrwLock);
        print_debug(self.rw_vec.as_ref().unwrap().id(), "FutureRwGuard::poll_lock: Called");

        // Move the queue along:
        unsafe { self.rw_vec.as_ref().unwrap().lock.process_queues(); }

        // Check for completion of the lock rx:
        if let Some(ref mut lock_rx) = self.lock_rx {
            match lock_rx.poll() {
                // If the poll returns `Async::Ready`, we have been popped from
                // the front of the lock queue and we now have exclusive access.
                // Otherwise, return the `NotReady`. The rx (oneshot channel) will
                // arrange for this task to be awakened when it's ready.
                Ok(status) => {
                    // if PRINT_DEBUG { println!("###### [{}] FutureRwGuard::poll_lock: status: {:?}, \
                    //     (thread: {}).", self.rw_vec.as_ref().unwrap().id(), status,
                    //     ::std::thread::current().name().unwrap_or("<unnamed>")); }
                    match status {
                        Async::Ready(_) => {
                            if let Some(ref lock_event) = self.lock_event {
                                lock_event.set_complete()?
                            }
                            self.stage = Stage::Command;
                        },
                        Async::NotReady => return Ok(Async::NotReady),
                    }
                },
                // Err(e) => return Err(e.into()),
                Err(e) => panic!("FutureRwGuard::poll_lock: {:?}", e),
            }
        } else {
            unreachable!();
        }

        self.poll_command()
    }


    /// Polls the lock until we have obtained a lock then polls the command
    /// event.
    #[cfg(feature = "async_block")]
    fn poll_lock(&mut self) -> AsyncResult<Async<G>> {
        debug_assert!(self.stage == Stage::QrwLock);
        print_debug(self.rw_vec.as_ref().unwrap().id(), "FutureRwGuard::poll_lock: Called");

        // Move the queue along:
        unsafe { self.rw_vec.as_ref().unwrap().lock.process_queues(); }

        // Wait until completion of the lock rx:
        self.lock_rx.take().wait()?;

        if let Some(ref lock_event) = self.lock_event {
            lock_event.set_complete()?
        }

        self.stage = Stage::Command;
        // if PRINT_DEBUG { println!("###### [{}] FutureRwGuard::poll_lock: Moving to command stage.",
        //     self.rw_vec.as_ref().unwrap().id()); }
        return self.poll_command();
    }

    /// Polls the command event until it is complete then returns an `RwGuard`
    /// which can be safely accessed immediately.
    fn poll_command(&mut self) -> AsyncResult<Async<G>> {
        debug_assert!(self.stage == Stage::Command);
        print_debug(self.rw_vec.as_ref().unwrap().id(), "FutureRwGuard::poll_command: Called");

        if let Some(ref mut command_completion) = self.command_completion {
            // if PRINT_DEBUG { println!("###### [{}] FutureRwGuard::poll_command: Polling command \
            //     completion event (thread: {}).", self.rw_vec.as_ref().unwrap().id(), ::std::thread::current().name()
            //     .unwrap_or("<unnamed>")); }

            if let Async::NotReady = command_completion.poll()? {
                return Ok(Async::NotReady);
            }
        }

        // Set cmd event to `None` so it doesn't get waited on unnecessarily
        // when this `FutureRwGuard` drops.
        self.command_completion = None;

        if self.upgrade_after_command {
            self.stage = Stage::Upgrade;
            self.poll_upgrade()
        } else {
            Ok(Async::Ready(self.into_guard()))
        }
    }

    /// Polls the lock until it has been upgraded.
    ///
    /// Only used if `::upgrade_after_command` has been called.
    ///
    #[cfg(not(feature = "async_block"))]
    fn poll_upgrade(&mut self) -> AsyncResult<Async<G>> {
        debug_assert!(self.stage == Stage::Upgrade);
        debug_assert!(self.upgrade_after_command);
        print_debug(self.rw_vec.as_ref().unwrap().id(), "FutureRwGuard::poll_upgrade: Called");

        // unsafe { self.rw_vec.as_ref().unwrap().lock.process_queues() }

        if self.upgrade_rx.is_none() {
            match unsafe { self.rw_vec.as_ref().unwrap().lock.upgrade_read_lock() } {
                Ok(_) => {
                    print_debug(self.rw_vec.as_ref().unwrap().id(),
                        "FutureRwGuard::poll_upgrade: Write lock acquired. Upgrading immediately.");
                    Ok(Async::Ready(self.into_guard()))
                },
                Err(rx) => {
                    self.upgrade_rx = Some(rx);
                    match self.upgrade_rx.as_mut().unwrap().poll() {
                        Ok(res) => {
                            // print_debug(self.rw_vec.as_ref().unwrap().id(),
                            //     "FutureRwGuard::poll_upgrade: Channel completed. Upgrading.");
                            // Ok(res.map(|_| self.into_guard()))
                            match res {
                                Async::Ready(_) => {
                                    print_debug(self.rw_vec.as_ref().unwrap().id(),
                                        "FutureRwGuard::poll_upgrade: Channel completed. Upgrading.");
                                    Ok(Async::Ready(self.into_guard()))
                                },
                                Async::NotReady => {
                                    print_debug(self.rw_vec.as_ref().unwrap().id(),
                                        "FutureRwGuard::poll_upgrade: Upgrade rx not ready.");
                                    Ok(Async::NotReady)
                                },
                            }
                        },
                        // Err(e) => Err(e.into()),
                        Err(e) => panic!("FutureRwGuard::poll_upgrade: {:?}", e),
                   }
                },
            }
        } else {
            // Check for completion of the upgrade rx:
            match self.upgrade_rx.as_mut().unwrap().poll() {
                Ok(status) => {
                    print_debug(self.rw_vec.as_ref().unwrap().id(),
                        &format!("FutureRwGuard::poll_upgrade: Status: {:?}", status));
                    Ok(status.map(|_| self.into_guard()))
                },
                // Err(e) => Err(e.into()),
                Err(e) => panic!("FutureRwGuard::poll_upgrade: {:?}", e),
            }
        }
    }

    /// Polls the lock until it has been upgraded.
    ///
    /// Only used if `::upgrade_after_command` has been called.
    ///
    #[cfg(feature = "async_block")]
    fn poll_upgrade(&mut self) -> AsyncResult<Async<G>> {
        debug_assert!(self.stage == Stage::Upgrade);
        debug_assert!(self.upgrade_after_command);
        print_debug(self.rw_vec.as_ref().unwrap().id(), "FutureRwGuard::poll_upgrade: Called");

        match unsafe { self.rw_vec.as_ref().unwrap().lock.upgrade_read_lock() } {
            Ok(_) => Ok(Async::Ready(self.into_guard())),
            Err(rx) => {
                self.upgrade_rx = Some(rx);
                self.upgrade_rx.take().unwrap().wait()?;
                Ok(Async::Ready(self.into_guard()))
            }
        }
    }

    /// Resolves this `FutureRwGuard` into the appropriate result guard.
    fn into_guard(&mut self) -> G {
        print_debug(self.rw_vec.as_ref().unwrap().id(), "FutureRwGuard::into_guard: All polling complete");
        G::new(self.rw_vec.take().unwrap(), self.release_event.take())
    }
}

impl<T, G> Future for FutureRwGuard<T, G> where G: RwGuard<T> {
    type Item = G;
    type Error = AsyncError;

    #[inline]
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.rw_vec.is_some() {
            match self.stage {
                Stage::Marker => self.poll_wait_events(),
                Stage::QrwLock => self.poll_lock(),
                Stage::Command => self.poll_command(),
                Stage::Upgrade => self.poll_upgrade(),
            }
        } else {
            Err("FutureRwGuard::poll: Task already completed.".into())
        }
    }
}

impl<T, G> Drop for FutureRwGuard<T, G> {
    /// Drops this FutureRwGuard.
    ///
    /// Blocks the current thread until the command associated with this
    /// `FutureRwGuard` (represented by the command completion event)
    /// completes. This ensures that the underlying `Vec` is not dropped
    /// before the command completes (which would cause obvious problems).
    fn drop(&mut self) {
        if let Some(ref ccev) = self.command_completion {
            // println!("###### FutureRwGuard::drop: Event ({:?}) incomplete...", ccev);
            // panic!("###### FutureRwGuard::drop: Event ({:?}) incomplete...", ccev);
            ccev.wait_for().expect("Error waiting on command completion event \
                while dropping 'FutureRwGuard'");
        }
        if let Some(ref rev) = self.release_event {
            rev.set_complete().expect("Error setting release event complete \
                while dropping 'FutureRwGuard'");
        }
    }
}

// a.k.a. FutureRead<T>
impl<T> FutureRwGuard<T, ReadGuard<T>> {
    pub fn upgrade_after_command(self) -> FutureWriteGuard<T> {
        use std::ptr::read;

        let future_guard = unsafe {
            FutureRwGuard {
                rw_vec: read(&self.rw_vec),
                lock_rx: read(&self.lock_rx),
                wait_list: read(&self.wait_list),
                lock_event: read(&self.lock_event),
                upgrade_after_command: true,
                upgrade_rx: None,
                command_completion: read(&self.command_completion),
                release_event: read(&self.release_event),
                stage: read(&self.stage),
                _guard: PhantomData,
            }
        };

        ::std::mem::forget(self);

        future_guard
    }
}


/// A locking `Vec` which interoperates with OpenCL events and Rust futures to
/// provide exclusive access to data.
///
/// Calling `::read` or `::write` returns a future which will resolve into a
/// `RwGuard`.
///
/// ## Platform Compatibility
///
/// Some CPU device/platform combinations have synchronization problems when
/// accessing an `RwVec` from multiple threads. Known platforms with problems
/// are 2nd and 4th gen Intel Core processors (Sandy Bridge and Haswell) with
/// Intel OpenCL CPU drivers. Others may be likewise affected. Run the
/// `device_check.rs` example to determine if your device/platform is
/// affected. AMD platform drivers are known to work properly on the
/// aforementioned CPUs so use those instead if possible.
#[derive(Debug)]
pub struct RwVec<T> {
    lock: QrwLock<Vec<T>>,
}

impl<T> RwVec<T> {
    /// Creates and returns a new `RwVec`.
    #[inline]
    pub fn new() -> RwVec<T> {
        RwVec {
            lock: QrwLock::new(Vec::new())
        }
    }

    /// Returns a new `FutureRwGuard` which will resolve into a a `RwGuard`.
    pub fn read(self) -> FutureReadGuard<T> {
        print_debug(self.id(), "RwVec::read: Read lock requested");
        let (tx, rx) = oneshot::channel();
        unsafe { self.lock.enqueue_lock_request(QrwRequest::new(tx, RequestKind::Read)); }
        FutureRwGuard::new(self.into(), rx)
    }

    /// Returns a new `FutureRwGuard` which will resolve into a a `RwGuard`.
    pub fn write(self) -> FutureWriteGuard<T> {
        print_debug(self.id(), "RwVec::write: Write lock requested");
        let (tx, rx) = oneshot::channel();
        unsafe { self.lock.enqueue_lock_request(QrwRequest::new(tx, RequestKind::Write)); }
        FutureRwGuard::new(self.into(), rx)
    }

    /// Returns a mutable slice into the contained `Vec`.
    ///
    /// Used by buffer command builders when preparing future read and write
    /// commands.
    ///
    /// Do not use unless you are 100% certain that there will be no other
    /// reads or writes for the entire access duration (only possible if
    /// manually manipulating the lock status).
    pub unsafe fn as_mut_slice(&self) -> &mut [T] {
        let ptr = (*self.lock.as_mut_ptr()).as_mut_ptr();
        let len = (*self.lock.as_ptr()).len();
        ::std::slice::from_raw_parts_mut(ptr, len)
    }

    /// Returns the length of the internal `Vec`.
    pub fn len(&self) -> usize {
        unsafe { (*self.lock.as_ptr()).len() }
    }

    /// Returns a pointer address to the internal array, usable as a unique
    /// identifier.
    ///
    /// Note that resizing the `Vec` will likely change the address. Also, the
    /// same 'id' could be reused by another `RwVec` created after this one is
    /// dropped.
    pub fn id(&self) -> usize {
        unsafe { (*self.lock.as_ptr()).as_ptr() as usize }
    }
}

impl<T> From<QrwLock<Vec<T>>> for RwVec<T> {
    fn from(q: QrwLock<Vec<T>>) -> RwVec<T> {
        RwVec { lock: q }
    }
}

impl<T> From<Vec<T>> for RwVec<T> {
    fn from(vec: Vec<T>) -> RwVec<T> {
        RwVec { lock: QrwLock::new(vec) }
    }
}

impl<T> Clone for RwVec<T> {
    #[inline]
    fn clone(&self) -> RwVec<T> {
        RwVec {
            lock: self.lock.clone(),
        }
    }
}