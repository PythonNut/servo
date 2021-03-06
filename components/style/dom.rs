/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![allow(unsafe_code)]

use data::PrivateStyleData;
use properties::{ComputedValues, PropertyDeclaration, PropertyDeclarationBlock};
use restyle_hints::{ElementSnapshot, RESTYLE_DESCENDANTS, RESTYLE_LATER_SIBLINGS, RESTYLE_SELF, RestyleHint};
use selectors::matching::DeclarationBlock;
use selectors::states::ElementState;
use smallvec::VecLike;
use std::cell::{Ref, RefMut};
use std::marker::PhantomData;
use std::ops::BitOr;
use std::sync::Arc;
use string_cache::{Atom, Namespace};

/// Opaque type stored in type-unsafe work queues for parallel layout.
/// Must be transmutable to and from TNode.
pub type UnsafeNode = (usize, usize);

/// An opaque handle to a node, which, unlike UnsafeNode, cannot be transformed
/// back into a non-opaque representation. The only safe operation that can be
/// performed on this node is to compare it to another opaque handle or to another
/// OpaqueNode.
///
/// Layout and Graphics use this to safely represent nodes for comparison purposes.
/// Because the script task's GC does not trace layout, node data cannot be safely stored in layout
/// data structures. Also, layout code tends to be faster when the DOM is not being accessed, for
/// locality reasons. Using `OpaqueNode` enforces this invariant.
#[derive(Clone, PartialEq, Copy, Debug, HeapSizeOf, Hash, Eq, Deserialize, Serialize)]
pub struct OpaqueNode(pub usize);

impl OpaqueNode {
    /// Returns the address of this node, for debugging purposes.
    #[inline]
    pub fn id(&self) -> usize {
        let OpaqueNode(pointer) = *self;
        pointer
    }
}

pub trait TRestyleDamage : BitOr<Output=Self> + Copy {
    fn compute(old: &Option<Arc<ComputedValues>>, new: &ComputedValues) -> Self;
    fn rebuild_and_reflow() -> Self;
}

pub trait TNode<'ln> : Sized + Copy + Clone {
    type ConcreteElement: TElement<'ln, ConcreteNode = Self, ConcreteDocument = Self::ConcreteDocument>;
    type ConcreteDocument: TDocument<'ln, ConcreteNode = Self, ConcreteElement = Self::ConcreteElement>;
    type ConcreteRestyleDamage: TRestyleDamage;

    fn to_unsafe(&self) -> UnsafeNode;
    unsafe fn from_unsafe(n: &UnsafeNode) -> Self;

    /// Returns whether this is a text node. It turns out that this is all the style system cares
    /// about, and thus obviates the need to compute the full type id, which would be expensive in
    /// Gecko.
    fn is_text_node(&self) -> bool;

    fn is_element(&self) -> bool;

    fn dump(self);

    fn traverse_preorder(self) -> TreeIterator<'ln, Self> {
        TreeIterator::new(self)
    }

    /// Returns an iterator over this node's children.
    fn children(self) -> ChildrenIterator<'ln, Self> {
        ChildrenIterator {
            current: self.first_child(),
            phantom: PhantomData,
        }
    }

    fn rev_children(self) -> ReverseChildrenIterator<'ln, Self> {
        ReverseChildrenIterator {
            current: self.last_child(),
            phantom: PhantomData,
        }
    }

    /// Converts self into an `OpaqueNode`.
    fn opaque(&self) -> OpaqueNode;

    /// Initializes style and layout data for the node. No-op if the data is already
    /// initialized.
    ///
    /// FIXME(pcwalton): Do this as part of fragment building instead of in a traversal.
    fn initialize_data(self);

    /// While doing a reflow, the node at the root has no parent, as far as we're
    /// concerned. This method returns `None` at the reflow root.
    fn layout_parent_node(self, reflow_root: OpaqueNode) -> Option<Self>;

    fn debug_id(self) -> usize;

    fn as_element(&self) -> Option<Self::ConcreteElement>;

    fn as_document(&self) -> Option<Self::ConcreteDocument>;

    fn children_count(&self) -> u32;

    fn has_changed(&self) -> bool;

    unsafe fn set_changed(&self, value: bool);

    fn is_dirty(&self) -> bool;

    unsafe fn set_dirty(&self, value: bool);

    fn has_dirty_descendants(&self) -> bool;

    unsafe fn set_dirty_descendants(&self, value: bool);

    fn dirty_self(&self) {
        unsafe {
            self.set_dirty(true);
            self.set_dirty_descendants(true);
        }
    }

    fn dirty_descendants(&self) {
        for ref child in self.children() {
            child.dirty_self();
            child.dirty_descendants();
        }
    }

    /// Borrows the PrivateStyleData without checks.
    #[inline(always)]
    unsafe fn borrow_data_unchecked(&self) -> Option<*const PrivateStyleData>;

    /// Borrows the PrivateStyleData immutably. Fails on a conflicting borrow.
    #[inline(always)]
    fn borrow_data(&self) -> Option<Ref<PrivateStyleData>>;

    /// Borrows the PrivateStyleData mutably. Fails on a conflicting borrow.
    #[inline(always)]
    fn mutate_data(&self) -> Option<RefMut<PrivateStyleData>>;

    /// Get the description of how to account for recent style changes.
    fn restyle_damage(self) -> Self::ConcreteRestyleDamage;

    /// Set the restyle damage field.
    fn set_restyle_damage(self, damage: Self::ConcreteRestyleDamage);

    fn parent_node(&self) -> Option<Self>;

    fn first_child(&self) -> Option<Self>;

    fn last_child(&self) -> Option<Self>;

    fn prev_sibling(&self) -> Option<Self>;

    fn next_sibling(&self) -> Option<Self>;


    /// Returns the style results for the given node. If CSS selector matching
    /// has not yet been performed, fails.
    fn style(&self) -> Ref<Arc<ComputedValues>>;

    /// Removes the style from this node.
    fn unstyle(self);
}

pub trait TDocument<'ld> : Sized + Copy + Clone {
    type ConcreteNode: TNode<'ld, ConcreteElement = Self::ConcreteElement, ConcreteDocument = Self>;
    type ConcreteElement: TElement<'ld, ConcreteNode = Self::ConcreteNode, ConcreteDocument = Self>;

    fn as_node(&self) -> Self::ConcreteNode;

    fn root_node(&self) -> Option<Self::ConcreteNode>;

    fn drain_modified_elements(&self) -> Vec<(Self::ConcreteElement, ElementSnapshot)>;
}

pub trait TElement<'le> : Sized + Copy + Clone + ::selectors::Element {
    type ConcreteNode: TNode<'le, ConcreteElement = Self, ConcreteDocument = Self::ConcreteDocument>;
    type ConcreteDocument: TDocument<'le, ConcreteNode = Self::ConcreteNode, ConcreteElement = Self>;

    fn as_node(&self) -> Self::ConcreteNode;

    fn style_attribute(&self) -> &'le Option<PropertyDeclarationBlock>;

    fn get_state(&self) -> ElementState;

    fn synthesize_presentational_hints_for_legacy_attributes<V>(&self, &mut V)
        where V: VecLike<DeclarationBlock<Vec<PropertyDeclaration>>>;

    fn get_attr<'a>(&'a self, namespace: &Namespace, attr: &Atom) -> Option<&'a str>;
    fn get_attrs<'a>(&'a self, attr: &Atom) -> Vec<&'a str>;

    /// Properly marks nodes as dirty in response to restyle hints.
    fn note_restyle_hint(&self, mut hint: RestyleHint) {
        // Bail early if there's no restyling to do.
        if hint.is_empty() {
            return;
        }

        // If the restyle hint is non-empty, we need to restyle either this element
        // or one of its siblings. Mark our ancestor chain as having dirty descendants.
        let node = self.as_node();
        let mut curr = node;
        while let Some(parent) = curr.parent_node() {
            if parent.has_dirty_descendants() { break }
            unsafe { parent.set_dirty_descendants(true); }
            curr = parent;
        }

        // Process hints.
        if hint.contains(RESTYLE_SELF) {
            node.dirty_self();

            // FIXME(bholley, #8438): We currently need to RESTYLE_DESCENDANTS in the
            // RESTYLE_SELF case in order to make sure "inherit" style structs propagate
            // properly. See the explanation in the github issue.
            hint.insert(RESTYLE_DESCENDANTS);
        }
        if hint.contains(RESTYLE_DESCENDANTS) {
            unsafe { node.set_dirty_descendants(true); }
            node.dirty_descendants();
        }
        if hint.contains(RESTYLE_LATER_SIBLINGS) {
            let mut next = ::selectors::Element::next_sibling_element(self);
            while let Some(sib) = next {
                let sib_node = sib.as_node();
                sib_node.dirty_self();
                sib_node.dirty_descendants();
                next = ::selectors::Element::next_sibling_element(&sib);
            }
        }
    }
}

pub struct TreeIterator<'a, ConcreteNode> where ConcreteNode: TNode<'a> {
    stack: Vec<ConcreteNode>,
    // Satisfy the compiler about the unused lifetime.
    phantom: PhantomData<&'a ()>,
}

impl<'a, ConcreteNode> TreeIterator<'a, ConcreteNode> where ConcreteNode: TNode<'a> {
    fn new(root: ConcreteNode) -> TreeIterator<'a, ConcreteNode> {
        let mut stack = vec!();
        stack.push(root);
        TreeIterator {
            stack: stack,
            phantom: PhantomData,
        }
    }
}

impl<'a, ConcreteNode> Iterator for TreeIterator<'a, ConcreteNode>
                                where ConcreteNode: TNode<'a> {
    type Item = ConcreteNode;
    fn next(&mut self) -> Option<ConcreteNode> {
        let ret = self.stack.pop();
        ret.map(|node| self.stack.extend(node.rev_children()));
        ret
    }
}

pub struct ChildrenIterator<'a, ConcreteNode> where ConcreteNode: TNode<'a> {
    current: Option<ConcreteNode>,
    // Satisfy the compiler about the unused lifetime.
    phantom: PhantomData<&'a ()>,
}

impl<'a, ConcreteNode> Iterator for ChildrenIterator<'a, ConcreteNode>
                                where ConcreteNode: TNode<'a> {
    type Item = ConcreteNode;
    fn next(&mut self) -> Option<ConcreteNode> {
        let node = self.current;
        self.current = node.and_then(|node| node.next_sibling());
        node
    }
}

pub struct ReverseChildrenIterator<'a, ConcreteNode> where ConcreteNode: TNode<'a> {
    current: Option<ConcreteNode>,
    // Satisfy the compiler about the unused lifetime.
    phantom: PhantomData<&'a ()>,
}

impl<'a, ConcreteNode> Iterator for ReverseChildrenIterator<'a, ConcreteNode>
                                where ConcreteNode: TNode<'a> {
    type Item = ConcreteNode;
    fn next(&mut self) -> Option<ConcreteNode> {
        let node = self.current;
        self.current = node.and_then(|node| node.prev_sibling());
        node
    }
}
