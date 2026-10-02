// Build script: generates the C++ side of the CXX-Qt bridge and compiles the QML
// module into the binary. Requires Qt 6 (qmake6 on PATH, or QMAKE=/path/to/qmake6).

use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("org.bogd3v.lampyris").qml_files(["qml/main.qml"]))
        .qt_module("Quick")
        .files(["src/controller.rs"])
        .build();
}
