pub(in self) use crate::logging::klog;

pub(in self) mod sealed;
pub(in self) use sealed::{PageFrameAllocatorImpl,IPageTableImpl,PAllocItem,PartialPageAllocation};

#[allow(private_bounds)]
pub trait PageFrameAllocator: PageFrameAllocatorImpl {}
#[allow(private_bounds)]
impl<T: PageFrameAllocatorImpl> PageFrameAllocator for T {}
#[allow(private_bounds)]
pub trait IPageTable: IPageTableImpl {}
#[allow(private_bounds)]
impl<T: IPageTableImpl> IPageTable for T {}


crate::arch_specific_module!(pub mod arch);
pub use arch::{canonical_addr,crop_addr,ptaddr_virt_to_phys,MIN_PAGE_SIZE};

mod allocators;
use allocators::firstfit as impl_firstfit;
use allocators::nodeeper as impl_nodeeper;
use impl_nodeeper::NoDeeper;

pub mod api;
pub use api::*;

pub mod strategy;
pub use strategy::*;

pub mod global_pages;
