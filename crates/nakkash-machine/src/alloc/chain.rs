//! The intrusive stack the block store keeps each class's blocks on: a
//! kept block's first bytes hold its [`Node`], the link to the block below
//! and the tick it was freed at, so keeping a block allocates nothing.

use std::ptr::NonNull;

/// The head of a kept block: the link to the block below it on its class's
/// stack, and the tick it was freed at.
#[repr(C)]
pub(super) struct Node {
    /// The block below, or null at the bottom.
    pub(super) next: *mut Node,
    /// The store's clock (its ticks) when it was freed.
    pub(super) freed: u64,
}

impl Node {
    /// Writes a node freed at tick `freed`, linked to nothing, at the start
    /// of `block`.
    ///
    /// # Safety
    /// `block` is valid for writes of a node, aligned to 8 bytes, and
    /// nothing else reads or writes its first bytes while the node is
    /// there.
    pub(super) unsafe fn at(block: *mut u8, freed: u64) -> NonNull<Node> {
        let node = block.cast::<Node>();
        let head = Node {
            next: std::ptr::null_mut(),
            freed,
        };
        // SAFETY: the caller's block, valid and aligned for a node.
        unsafe { node.write(head) };
        // SAFETY: valid for writes, so not null.
        unsafe { NonNull::new_unchecked(node) }
    }
}

/// A stack of nodes linked through their `next`, top first.
pub(super) struct Chain {
    /// The top node, or null when empty.
    head: *mut Node,
}

// SAFETY: a chain owns the blocks it links: no other thread reads or
// writes them while the chain holds them, so it may move between threads.
unsafe impl Send for Chain {}

impl Chain {
    /// An empty chain.
    pub(super) const fn new() -> Self {
        Chain {
            head: std::ptr::null_mut(),
        }
    }

    /// Whether the chain holds no node.
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "read by `has` on Linux, and by the tests")
    )]
    pub(super) fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    /// Puts `node` on top.
    ///
    /// # Safety
    /// `node` is valid and in no chain, and stays valid, read and written
    /// only by this chain, until it is popped.
    pub(super) unsafe fn push(&mut self, node: NonNull<Node>) {
        // SAFETY: the caller's node, which this chain alone writes.
        unsafe { (*node.as_ptr()).next = self.head };
        self.head = node.as_ptr();
    }

    /// Takes the top node off, if any.
    pub(super) fn pop(&mut self) -> Option<NonNull<Node>> {
        let node = NonNull::new(self.head)?;
        // SAFETY: a node `push` was given, valid while the chain holds it.
        self.head = unsafe { node.as_ref() }.next;
        Some(node)
    }

    /// Takes every node `gone` picks off into a chain of its own, and keeps
    /// the others in their order.
    pub(super) fn split_off(&mut self, gone: impl Fn(&Node) -> bool) -> Chain {
        let mut picked = Chain::new();
        let mut rest = Chain::new();
        while let Some(node) = self.pop() {
            // SAFETY: just popped, so valid and held by no chain.
            let to = match gone(unsafe { node.as_ref() }) {
                true => &mut picked,
                false => &mut rest,
            };
            // SAFETY: popped from this chain, now in no chain.
            unsafe { to.push(node) };
        }
        // `rest` is upside down; turned back, the newest is on top again.
        while let Some(node) = rest.pop() {
            // SAFETY: popped from `rest`, now in no chain.
            unsafe { self.push(node) };
        }
        picked
    }
}

#[cfg(test)]
#[path = "../../tests/unit/alloc/chain.rs"]
mod tests;
