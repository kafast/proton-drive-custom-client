#include "protonsync-gui-qt/cpp/application.h"

namespace {
// QApplication verlangt argc/argv, die so lange leben wie die Anwendung.
int argument_count = 1;
char program_name[] = "protonsync-tray";
char* arguments[] = { program_name, nullptr };
}

std::unique_ptr<QApplication> new_application()
{
    auto app = std::make_unique<QApplication>(argument_count, arguments);
    QApplication::setApplicationName(QStringLiteral("protonsync-tray"));
    QApplication::setOrganizationName(QStringLiteral("protonsync"));
    // Das Fenster zu schließen beendet die App nicht; sie lebt im Tray weiter.
    QApplication::setQuitOnLastWindowClosed(false);
    return app;
}

std::int32_t run_application(QApplication& app)
{
    return app.exec();
}
