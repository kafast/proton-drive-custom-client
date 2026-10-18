// Creates the Qt application. QApplication (Qt Widgets) instead of QGuiApplication,
// so that the tray icon also appears without a StatusNotifier service (ADR 0011).
#pragma once

#include <QtWidgets/QApplication>
#include <cstdint>
#include <memory>

std::unique_ptr<QApplication> new_application();
std::int32_t run_application(QApplication& app);
