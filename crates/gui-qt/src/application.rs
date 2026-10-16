//! Brücke zu `cpp/application.cpp`: die Qt-Anwendung anlegen und ausführen.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("protonsync-gui-qt/cpp/application.h");

        /// Die Qt-Anwendung (mit Qt Widgets).
        type QApplication;

        /// Legt die Anwendung an; nur einmal pro Prozess aufrufen.
        fn new_application() -> UniquePtr<QApplication>;

        /// Führt die Ereignisschleife aus, bis die App beendet wird.
        fn run_application(app: Pin<&mut QApplication>) -> i32;
    }
}
