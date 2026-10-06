//! Builds the Linux scanner sources against fixture sysfs trees on any Unix
//! host, so its parsers and section collectors are exercised (and linted) on
//! macOS development machines too. On Linux the same tests also run as unit
//! tests of the library.
#![cfg(unix)]
#![allow(dead_code)]

mod contracts {
    pub use app_lib::contracts::*;
}

mod error {
    pub use app_lib::error::*;
}

mod tasks {
    pub mod cancellation {
        pub use app_lib::tasks::cancellation::*;
    }
}

#[path = "../src/platform"]
mod platform {
    pub mod common {
        pub use app_lib::platform::common::*;
    }

    #[path = "linux"]
    pub mod linux {
        pub mod acpi_dump;
        pub mod devices;
        pub mod input;
        pub mod parse;
        pub mod scanner;
        pub mod sysfs;
        pub mod system;
    }
}
