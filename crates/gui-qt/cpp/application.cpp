#include "protonsync-gui-qt/cpp/application.h"

namespace {
// QApplication requires argc/argv that live as long as the application.
int argument_count = 1;
char program_name[] = "protonsync-tray";
char* arguments[] = { program_name, nullptr };
}

std::unique_ptr<QApplication> new_application()
{
    auto app = std::make_unique<QApplication>(argument_count, arguments);
    QApplication::setApplicationName(QStringLiteral("protonsync-tray"));
    QApplication::setOrganizationName(QStringLiteral("protonsync"));
    // Closing the window does not quit the app; it keeps living in the tray.
    QApplication::setQuitOnLastWindowClosed(false);
    return app;
}

std::int32_t run_application(QApplication& app)
{
    return app.exec();
}
