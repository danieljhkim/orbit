//! Private persistence drivers. Drivers implement one storage technology and
//! never coordinate another driver.

pub(crate) mod file;
pub(crate) mod sqlite;
