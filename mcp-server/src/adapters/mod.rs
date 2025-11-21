// IronBase storage adapters

pub mod ironbase_adapter;
pub mod ironbase_real;

pub use ironbase_adapter::IronBaseAdapter;
pub use ironbase_real::RealIronBaseAdapter;

// Type alias for the adapter to use
// Switch between mock (dev) and real (production)
#[cfg(feature = "real-ironbase")]
pub type ActiveAdapter = RealIronBaseAdapter;

#[cfg(not(feature = "real-ironbase"))]
pub type ActiveAdapter = IronBaseAdapter;
