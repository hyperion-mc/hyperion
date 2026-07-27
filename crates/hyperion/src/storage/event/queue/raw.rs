use std::{
    any::TypeId, cell::SyncUnsafeCell, mem::MaybeUninit, ptr::NonNull, sync::atomic::AtomicUsize,
};

use anyhow::bail;

/// Denotes a pointer that will become invalid at the end of the tick (it is bump allocated)
#[derive(Debug, Copy, Clone)]
pub struct TypedBumpPtr {
    id: TypeId,
    // a ptr to a bump allocated event
    elem: NonNull<()>,
}

unsafe impl Send for TypedBumpPtr {}
unsafe impl Sync for TypedBumpPtr {}

impl TypedBumpPtr {
    #[must_use]
    pub const fn new(id: TypeId, elem: NonNull<()>) -> Self {
        Self { id, elem }
    }

    #[must_use]
    pub const fn id(&self) -> TypeId {
        self.id
    }

    #[must_use]
    pub const fn elem(&self) -> NonNull<()> {
        self.elem
    }
}

/// Think of this as a fixed capacity `Vec<T>`
pub struct RawQueue<T> {
    elems: Box<[SyncUnsafeCell<MaybeUninit<T>>]>,
    len: AtomicUsize,
}

// todo: remove Copy requirement.
impl<T: Copy> RawQueue<T> {
    #[must_use]
    pub fn new(size: usize) -> Self {
        let elems = (0..size)
            .map(|_| SyncUnsafeCell::new(MaybeUninit::uninit()))
            .collect();

        Self {
            elems,
            len: AtomicUsize::new(0),
        }
    }

    pub fn push(&self, elem: T) -> anyhow::Result<()> {
        let ptr = self.len.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let elems = &*self.elems;

        let Some(ptr) = elems.get(ptr) else {
            self.len.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            bail!("queue is full");
        };

        let ptr = unsafe { &mut *ptr.get() };
        ptr.write(elem);

        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        let len = self.len.load(std::sync::atomic::Ordering::Relaxed);

        (0..len).map(move |i| {
            let elem = &self.elems[i];
            let elem = unsafe { &*elem.get() };
            unsafe { elem.assume_init_read() }
        })
    }

    pub fn reset(&mut self) {
        // we do not need to `Drop` because NonNull does not implement Drop
        *self.len.get_mut() = 0;
    }
}
