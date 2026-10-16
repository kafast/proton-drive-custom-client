// Legt die Qt-Anwendung an. QApplication (Qt Widgets) statt QGuiApplication,
// damit das Tray-Icon auch ohne StatusNotifier-Dienst erscheint (ADR 0011).
#pragma once

#include <QtWidgets/QApplication>
#include <cstdint>
#include <memory>

std::unique_ptr<QApplication> new_application();
std::int32_t run_application(QApplication& app);
