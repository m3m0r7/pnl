//! C-world discovery and interop: native-library/header resolution, header
//! parsing, pkg-config, the C toolchain, and compiled shims. This is the only
//! layer allowed to touch libclang, pkg-config files, or the C compiler.

pub mod cc;
pub mod discovery;
pub mod header_adapter;
pub mod install_script;
pub mod pkg_config;
pub mod shim;
pub mod tbd;
