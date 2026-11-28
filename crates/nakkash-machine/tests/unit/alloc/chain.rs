use std::ptr::NonNull;

use super::*;

/// Nodes on the test's heap, freed at the epochs given: what a chain links
/// in the store's blocks.
fn nodes(freed: &[u64]) -> Vec<Node> {
    freed
        .iter()
        .map(|&freed| Node {
            next: std::ptr::null_mut(),
            freed,
        })
        .collect()
}

/// The epochs of a chain's nodes, top first, popping them all.
fn epochs(chain: &mut Chain) -> Vec<u64> {
    std::iter::from_fn(|| chain.pop())
        // SAFETY: the test's nodes outlive the chain.
        .map(|node| unsafe { node.as_ref() }.freed)
        .collect()
}

#[test]
fn a_chain_is_last_in_first_out() {
    let mut held = nodes(&[1, 2, 3]);
    let mut chain = Chain::new();
    assert!(chain.is_empty());
    for node in &mut held {
        // SAFETY: each node is pushed once and outlives the chain.
        unsafe { chain.push(NonNull::from(node)) };
    }
    assert!(!chain.is_empty());
    assert_eq!(epochs(&mut chain), [3, 2, 1]);
    assert!(chain.is_empty());
}

#[test]
fn splitting_off_stale_nodes_keeps_the_rest_in_order() {
    let mut held = nodes(&[1, 5, 2, 6, 3]);
    let mut chain = Chain::new();
    for node in &mut held {
        // SAFETY: each node is pushed once and outlives the chain.
        unsafe { chain.push(NonNull::from(node)) };
    }
    let mut gone = chain.split_off(|node| node.freed < 4);
    assert_eq!(epochs(&mut chain), [6, 5]);
    let mut gone_epochs = epochs(&mut gone);
    gone_epochs.sort_unstable();
    assert_eq!(gone_epochs, [1, 2, 3]);
}

#[test]
fn splitting_an_empty_chain_or_keeping_everything_changes_nothing() {
    let mut chain = Chain::new();
    assert!(chain.split_off(|_| true).is_empty());
    let mut held = nodes(&[1, 2]);
    for node in &mut held {
        // SAFETY: each node is pushed once and outlives the chain.
        unsafe { chain.push(NonNull::from(node)) };
    }
    assert!(chain.split_off(|_| false).is_empty());
    assert_eq!(epochs(&mut chain), [2, 1]);
}

#[test]
fn a_node_is_written_at_the_start_of_its_block() {
    let mut block = vec![0xABu64; 8];
    let at = block.as_mut_ptr().cast::<u8>();
    // SAFETY: the block holds 64 bytes, aligned to 8, and nothing else
    // reads it while the node is there.
    let node = unsafe { Node::at(at, 42) };
    assert_eq!(node.as_ptr().cast::<u8>(), at);
    // SAFETY: the node was just written.
    let node = unsafe { node.as_ref() };
    assert_eq!((node.freed, node.next), (42, std::ptr::null_mut()));
    assert_eq!(block[2..], [0xABu64; 6], "only the node's bytes written");
}
