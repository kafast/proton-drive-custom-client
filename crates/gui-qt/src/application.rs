//! Bridge to `cpp/application.cpp`: create and run the Qt application.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("protonsync-gui-qt/cpp/application.h");

        /// The Qt application (with Qt Widgets).
        type QApplication;

        /// Creates the application; call only once per process.
        fn new_application() -> UniquePtr<QApplication>;

        /// Runs the event loop until the app quits.
        fn run_application(app: Pin<&mut QApplication>) -> i32;
    }
}
