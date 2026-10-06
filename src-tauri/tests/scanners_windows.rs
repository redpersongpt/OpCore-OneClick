//! Builds the platform-independent parts of the Windows scanner (inventory
//! script and JSON mapping, Win32 buffer decoders) on every host, so they are
//! tested and linted outside Windows too. On Windows the same tests also run
//! as unit tests of the library.
#![allow(dead_code)]

mod contracts {
    pub use app_lib::contracts::*;
}

mod error {
    pub use app_lib::error::*;
}

#[path = "../src/platform"]
mod platform {
    pub mod common {
        pub use app_lib::platform::common::*;
    }

    #[path = "windows"]
    pub mod windows {
        pub mod inventory;
        pub mod raw;
    }
}
