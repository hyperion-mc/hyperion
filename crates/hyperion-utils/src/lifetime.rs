use std::{
    mem::{ManuallyDrop, size_of},
    sync::Mutex,
    thread::{self, Thread},
    time::{Duration, Instant},
};

use tracing::{info, warn};

/// # Safety
/// Same safety requirements as [`std::mem::transmute`]. In addition, both types must have the same
/// size, but this is not checked at compile time.
unsafe fn transmute_unchecked<Src, Dst>(src: Src) -> Dst {
    debug_assert_eq!(size_of::<Src>(), size_of::<Dst>());
    let src = ManuallyDrop::new(src);
    // SAFETY: ensured by caller
    unsafe { std::ptr::read(std::ptr::from_ref(&src).cast()) }
}

/// # Safety
/// The type of [`Lifetime::WithLifetime`] must be the same type as `Self` aside from lifetimes. In
/// addition, [`Lifetime::WithLifetime`] may not use `'static` in a lifetime parameter if the original
/// `Self` type did not use `'static` in the same lifetime parameters.
pub unsafe trait Lifetime {
    type WithLifetime<'a>: Lifetime + 'a;

    /// # Safety
    /// This may change references to have the lifetime of `'a`, which may impose additional safety
    /// requirements.
    #[must_use]
    unsafe fn change_lifetime<'a>(self) -> Self::WithLifetime<'a>
    where
        Self: Sized,
    {
        // SAFETY: This lifetime cast is checked by the caller, and the safety requirements on implementors of
        // the Lifetime trait ensure that no type cast or cast to a longer lifetime is occuring.
        unsafe { transmute_unchecked(self) }
    }

    fn shorten_lifetime<'a>(self) -> Self::WithLifetime<'a>
    where
        Self: 'a + Sized,
    {
        // SAFETY: Shortening a lifetime is allowed
        unsafe { self.change_lifetime() }
    }

    fn shorten_lifetime_ref<'a>(&self) -> &Self::WithLifetime<'a>
    where
        Self: 'a + Sized,
    {
        // SAFETY: Shortening a lifetime is allowed
        unsafe { &*std::ptr::from_ref(self).cast::<Self::WithLifetime<'a>>() }
    }
}

unsafe impl<T> Lifetime for &T
where
    T: ?Sized + 'static,
{
    type WithLifetime<'a> = &'a T;
}

hyperion_packet_macros::for_each_static_play_c2s_packet! {
    unsafe impl Lifetime for PACKET {
        type WithLifetime<'a> = PACKET;
    }
}

hyperion_packet_macros::for_each_lifetime_play_c2s_packet! {
    unsafe impl Lifetime for PACKET<'_> {
        type WithLifetime<'a> = PACKET<'a>;
    }
}

#[cfg(debug_assertions)]
struct Reference {
    trace: std::backtrace::Backtrace,
    ty: &'static str,
}

#[derive(Copy, Clone)]
struct ReferenceId(#[cfg(debug_assertions)] usize);

#[derive(Default)]
#[doc(hidden)]
pub struct References {
    waiting_thread: Mutex<Option<Thread>>,

    #[cfg(debug_assertions)]
    references: Mutex<Vec<Option<Reference>>>,
    #[cfg(not(debug_assertions))]
    reference_count: std::sync::atomic::AtomicUsize,
}

impl References {
    #[cfg(debug_assertions)]
    fn status(&self) -> String {
        use std::backtrace::BacktraceStatus;

        let references = self.references.lock().unwrap();
        if references.iter().any(Option::is_some) {
            references
                .iter()
                .flatten()
                .enumerate()
                .map(|(i, Reference { trace, ty })| match trace.status() {
                    BacktraceStatus::Disabled => {
                        format!(
                            "RuntimeLifetime #{i} to {ty} was not dropped yet. consider setting \
                             RUST_BACKTRACE=1 to show a backtrace of where the RuntimeLifetime \
                             was created."
                        )
                    }
                    BacktraceStatus::Captured => {
                        format!(
                            "RuntimeLifetime #{i} to {ty} was not dropped yet. the \
                             RuntimeLifetime was created at the following location:\n{trace}"
                        )
                    }
                    _ => {
                        format!(
                            "RuntimeLifetime #{i} to {ty} was not dropped yet. backtraces are not \
                             supported on the current platform."
                        )
                    }
                })
                .fold(String::new(), |a, b| a + "\n" + &b)
        } else {
            "no active references".to_string()
        }
    }

    #[cfg(not(debug_assertions))]
    fn status(&self) -> String {
        let references = self
            .reference_count
            .load(std::sync::atomic::Ordering::Relaxed);
        if references != 0 {
            format!(
                "{references} active references - consider compiling with debug_assertions \
                 enabled (such as by compiling in debug mode) for more debug information"
            )
        } else {
            "no active references".to_string()
        }
    }

    #[cfg(debug_assertions)]
    fn has_references(&self) -> bool {
        self.references.lock().unwrap().iter().any(Option::is_some)
    }

    #[cfg(not(debug_assertions))]
    fn has_references(&self) -> bool {
        self.reference_count
            .load(std::sync::atomic::Ordering::Relaxed)
            != 0
    }

    fn wait_no_references(&self) {
        // The waiting thread is set first before checking if there are any references to avoid
        // race conditions. If the last reference is dropped after the waiting thread is set, this
        // thread will be unparked. Even if this thread is unparked after the has_references check
        // returns true and before the call to park_timeout, park_timeout will return immediately.
        {
            let mut waiting_thread = self.waiting_thread.lock().unwrap();
            assert!(
                waiting_thread.is_none(),
                "cannot call wait_no_references from multiple threads"
            );
            *waiting_thread = Some(thread::current());
        }

        if self.has_references() {
            info!("blocking until there are no more references");
            let start = Instant::now();
            loop {
                thread::park_timeout(Duration::from_secs(5));
                if !self.has_references() {
                    break;
                }

                warn!(
                    "thread is blocked because there are still references after {:?}: {}",
                    start.elapsed(),
                    self.status()
                );
            }
            info!("there are no more references after {:?}", start.elapsed());
        }

        {
            let mut waiting_thread = self.waiting_thread.lock().unwrap();
            debug_assert_eq!(
                waiting_thread.as_ref().map(Thread::id),
                Some(thread::current().id())
            );
            *waiting_thread = None;
        }
    }

    #[cfg(debug_assertions)]
    fn acquire<T>(&self) -> ReferenceId {
        let mut references = self.references.lock().unwrap();
        let id = ReferenceId(references.len());
        references.push(Some(Reference {
            trace: std::backtrace::Backtrace::capture(),
            ty: std::any::type_name::<T>(),
        }));
        id
    }

    #[cfg(not(debug_assertions))]
    #[expect(clippy::extra_unused_type_parameters)]
    fn acquire<T>(&self) -> ReferenceId {
        // Relaxed ordering is used here because a shared reference is being held to the
        // LifetimeTracker, meaning that LifetimeTracker::wait_no_references cannot be called
        // concurrently in another thread becuase it requires an exclusive reference to the
        // LifetimeTracker. In a multi-threaded scenario where the LifetimeTracker is shared
        // across threads, there will always be a happens-before relationship where this increment
        // occurs before LifetimeTracker::wait_no_references is called and reads this value
        // because the synchronization primitive needed to get an exclusive reference to
        // LifetimeTracker should form a happens-before relationship, so using a stricter ordering
        // here is not needed.
        self.reference_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ReferenceId()
    }

    #[cfg(debug_assertions)]
    unsafe fn release(&self, id: ReferenceId) {
        let remaining;
        {
            let mut references = self.references.lock().unwrap();
            references[id.0] = None;
            remaining = references.iter().any(Option::is_some);

            if !remaining {
                // Clear the references to reduce memory usage; no more references exist so reference ids can be invalidated
                references.clear();
            }
        }

        if !remaining {
            let waiting_thread = self.waiting_thread.lock().unwrap();
            if let Some(waiting_thread) = &*waiting_thread {
                waiting_thread.unpark();
            }
        }
    }

    #[cfg(not(debug_assertions))]
    unsafe fn release(&self, _id: ReferenceId) {
        let remaining = self
            .reference_count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
            == 1;

        if !remaining {
            let waiting_thread = self.waiting_thread.lock().unwrap();
            if let Some(waiting_thread) = &*waiting_thread {
                waiting_thread.unpark();
            }
        }
    }
}

pub struct RuntimeLifetime<T> {
    value: T,
    references: *const References,
    id: ReferenceId,
}

impl<T: Lifetime> RuntimeLifetime<T> {
    #[must_use]
    pub fn new<'a>(
        value: T,
        handle: &dyn LifetimeHandle<'a>,
    ) -> RuntimeLifetime<T::WithLifetime<'static>>
    where
        T: 'a,
    {
        // SAFETY: RuntimeLifetime::get ensures that the 'static referencs are not exposed
        // publicly and that users can only access T with an appropriate lifetime.
        let value = unsafe { value.change_lifetime::<'static>() };

        let references = handle.__private_references(sealed::Sealed);
        let id = references.acquire::<T>();

        RuntimeLifetime {
            value,
            references: std::ptr::from_ref(references),
            id,
        }
    }

    #[must_use]
    pub const fn get<'a>(&'a self) -> &'a T::WithLifetime<'a> {
        // SAFETY: LifetimeTracker::wait_no_references will wait until `self` is dropped
        // before any references are invalidated. 'a will expire before self is
        // dropped. Therefore, it is safe to change these references to 'a.
        unsafe { &*(&raw const self.value).cast::<T::WithLifetime<'a>>() }
    }
}

unsafe impl<T> Send for RuntimeLifetime<T> where T: Send {}
unsafe impl<T> Sync for RuntimeLifetime<T> where T: Sync {}

impl<T> Drop for RuntimeLifetime<T> {
    fn drop(&mut self) {
        // SAFETY: `self.references` is safe to dereference because the underlying LifetimeTracker would
        // have already aborted if it were dropped before this
        let references = unsafe { &*self.references };

        unsafe { references.release(self.id) };

        // Dropping the inner value is sound despite having 'static lifetime parameters because
        // Drop implementations cannot be specialized, meaning that the Drop implementation cannot
        // change its behavior to do something unsound (such as by keeping those 'static references
        // after the value is dropped) when the type has 'static lifetime parameters.
    }
}

mod sealed {
    pub struct Sealed;
}

pub trait LifetimeHandle<'a> {
    #[must_use]
    #[doc(hidden)]
    fn __private_references(&self, _: sealed::Sealed) -> &References;
}

struct LifetimeHandleObject<'a> {
    references: &'a References,
}

impl<'a> LifetimeHandle<'a> for LifetimeHandleObject<'a> {
    fn __private_references(&self, _: sealed::Sealed) -> &References {
        self.references
    }
}

#[derive(Default)]
pub struct LifetimeTracker {
    references: Box<References>,
}

impl LifetimeTracker {
    pub fn wait_no_references(&mut self) {
        self.references.wait_no_references();
    }

    /// # Safety
    /// Data which outlives the `'handle` lifetime and might have a [`RuntimeLifetime`] constructed with the resulting
    /// [`LifetimeHandle`] must only be dropped after [`LifetimeTracker::wait_no_references`] is called on this
    /// tracker. The only purpose of the `'handle` lifetime is to allow users to control which values can be wrapped
    /// in a [`RuntimeLifetime`] since wrapped values must outlive `'handle`. The length of the `'handle` lifetime
    /// itself does not matter, and `'handle` may expire before [`LifetimeTracker::wait_no_references`] is called.
    #[must_use]
    pub unsafe fn handle<'handle>(&'handle self) -> impl LifetimeHandle<'handle> {
        // Storing the lifetime parameter in a trait (LifetimeHandle) instead of a struct is necessary to prohibit
        // casts to a shorter lifetime. If the LifetimeHandle's lifetime could be shortened, the user could safely
        // wrap values of any lifetime in RuntimeLifetime, which would defeat the purpose of the 'handle lifetime.
        LifetimeHandleObject::<'handle> {
            references: &self.references,
        }
    }
}

impl Drop for LifetimeTracker {
    fn drop(&mut self) {
        // Even if data associated with this tracker will live for 'static, the Box storing
        // the references will be dropped, so this ensures that there are no
        // RuntimeLifetimes which might still have a pointer to the references.
        self.wait_no_references();
    }
}
