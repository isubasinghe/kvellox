use std::cell::UnsafeCell;

// A transparent wrapper around UnsafeCell.
// implies: "I promise I am accessing this from one thread only."
#[repr(transparent)]
pub struct FastCell<T> {
    inner: UnsafeCell<T>,
}

impl<T> FastCell<T> {
    pub fn new(value: T) -> Self {
        Self { inner: UnsafeCell::new(value) }
    }

    // Get a mutable pointer. Zero cost (it's just a cast).
    // Useful if you are building linked lists/trees manually.
    #[inline(always)]
    pub fn as_ptr(&self) -> *mut T {
        self.inner.get()
    }

    // The dangerous one.
    // SAFETY: You must ensure no other &mut reference to this data exists
    // alive in the compiler's eyes (aliasing rules).
    #[inline(always)]
    pub unsafe fn get_mut(&self) -> &mut T {
        &mut *self.inner.get()
    }
    
    // For copy types, this is completely safe and essentially free.
    #[inline(always)]
    pub fn set(&self, val: T) {
        unsafe { *self.inner.get() = val; }
    }
    
    #[inline(always)]
    pub fn get(&self) -> T where T: Copy {
        unsafe { *self.inner.get() }
    }
}
