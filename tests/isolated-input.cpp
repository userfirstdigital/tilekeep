// Native input for a private KWin only. Never connects to the login desktop.
#include <QCoreApplication>
#include <QPointF>
#include <KWayland/Client/registry.h>
#include <KWayland/Client/fakeinput.h>
#include <wayland-client.h>
#include <sys/socket.h>
#include <unistd.h>
#include <iostream>
#include <sstream>
#include <memory>

int main(int argc, char **argv) {
    QCoreApplication app(argc, argv);
    const auto runtime = qEnvironmentVariable("XDG_RUNTIME_DIR");
    const auto root = qEnvironmentVariable("TILEKEEP_ISOLATED_DIR");
    const auto socket = qEnvironmentVariable("WAYLAND_DISPLAY");
    const auto pid = qEnvironmentVariableIntValue("TILEKEEP_ISOLATED_KWIN_PID");
    if (!root.startsWith("/tmp/tilekeep-isolated-") || runtime != root + "/runtime" || socket != "tilekeep-isolated" || pid <= 0) return 2;
    auto *display = wl_display_connect(socket.toUtf8().constData());
    if (!display) return 3;
    ucred peer{}; socklen_t size = sizeof(peer);
    if (getsockopt(wl_display_get_fd(display), SOL_SOCKET, SO_PEERCRED, &peer, &size) || peer.pid != pid || peer.uid != getuid()) return 4;
    KWayland::Client::Registry registry;
    std::unique_ptr<KWayland::Client::FakeInput> input;
    QObject::connect(&registry, &KWayland::Client::Registry::fakeInputAnnounced, [&](quint32 name, quint32 version) {
        input.reset(registry.createFakeInput(name, version));
    });
    registry.create(display); registry.setup();
    if (wl_display_roundtrip(display) < 0 || !input) return 5;
    input->authenticate("Tilekeep isolated test", "Native drag regression in a disposable compositor");
    wl_display_roundtrip(display);
    std::cout << "READY" << std::endl;
    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream command(line); std::string op; double x, y; unsigned key, state;
        command >> op;
        if (op == "move" && command >> x >> y) input->requestPointerMoveAbsolute(QPointF(x, y));
        else if (op == "button" && command >> state) {
            if (state) input->requestPointerButtonPress(Qt::LeftButton); else input->requestPointerButtonRelease(Qt::LeftButton);
        } else if (op == "key" && command >> key >> state) {
            if (state) input->requestKeyboardKeyPress(key); else input->requestKeyboardKeyRelease(key);
        } else if (op == "quit") break;
        else return 6;
        if (wl_display_roundtrip(display) < 0) return 7;
        std::cout << "OK" << std::endl;
    }
    input.reset(); registry.release(); wl_display_disconnect(display);
}
